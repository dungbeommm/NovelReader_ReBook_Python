use anyhow::{bail, Context, Result};
use quick_xml::{events::Event, Reader};
use regex::Regex;
use serde_json::Value;
use std::{collections::HashMap, io::{Cursor, Read, Write}};
use zip::{read::ZipArchive, write::FileOptions, CompressionMethod, ZipWriter};

use crate::text::{decode_txt, normalize, split_chapters, MAX_FILE};

const MAX_EPUB_EXPANDED: u64 = 200 * 1024 * 1024;
const MAX_EPUB_ENTRIES: usize = 20_000;

pub fn parse_file(name: &str, data: &[u8]) -> Result<(Vec<(String, String)>, String)> {
    anyhow::ensure!(data.len() <= MAX_FILE, "File tối đa 50 MB.");
    match std::path::Path::new(name).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "txt" => {
            let (text, encoding) = decode_txt(data)?;
            Ok((split_chapters(&text)?, encoding))
        }
        "epub" => Ok((parse_epub(data)?, "EPUB / UTF-8".into())),
        _ => bail!("Chỉ hỗ trợ .txt và .epub."),
    }
}

fn safe_epub_name(name: &str) -> bool {
    !name.starts_with('/') && !name.starts_with('\\') && !name.contains('\\') &&
        name.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn xml_attr(start: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    start.attributes().flatten().find(|a| a.key.as_ref() == key)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

fn xml_text(input: &str) -> Result<Vec<(String, String)>> {
    let mut reader = Reader::from_str(input);
    reader.trim_text(false);
    let mut buf = Vec::new();
    let mut output = String::new();
    let mut title = String::new();
    let mut title_depth = 0usize;
    let mut skip_depth = 0usize;
    let mut skip_tag = false;
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                depth += 1;
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase();
                if skip_depth > 0 { skip_depth += 1; }
                else if matches!(tag.as_str(), "script" | "style" | "nav" | "svg") {
                    skip_depth = 1;
                } else {
                    if matches!(tag.as_str(), "h1" | "h2" | "h3") && title.is_empty() { title_depth = depth; }
                    if matches!(tag.as_str(), "p" | "div" | "h1" | "h2" | "h3" | "li" | "blockquote" | "br") {
                        output.push('\n');
                    }
                }
            }
            Event::Empty(e) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase();
                if skip_depth == 0 && matches!(tag.as_str(), "br" | "p" | "div") { output.push('\n'); }
            }
            Event::Text(e) if skip_depth == 0 => {
                let value = e.unescape()?.into_owned();
                if title_depth > 0 && depth >= title_depth { title.push_str(&value); }
                output.push_str(&value);
            }
            Event::CData(e) if skip_depth == 0 => {
                let value = String::from_utf8_lossy(e.as_ref());
                if title_depth > 0 && depth >= title_depth { title.push_str(&value); }
                output.push_str(&value);
            }
            Event::End(e) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase();
                if skip_depth > 0 { skip_depth -= 1; }
                else if matches!(tag.as_str(), "p" | "div" | "h1" | "h2" | "h3" | "li" | "blockquote") {
                    output.push('\n');
                }
                if title_depth == depth && matches!(tag.as_str(), "h1" | "h2" | "h3") { title_depth = 0; }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let body = normalize(&output);
    if body.is_empty() { return Ok(Vec::new()); }
    let clean_title = normalize(&title);
    let parsed = split_chapters(&body);
    if let Ok(chapters) = parsed {
        if chapters.len() > 1 { return Ok(chapters); }
    }
    let final_title = if clean_title.is_empty() { "Phần nội dung".into() } else { clean_title };
    Ok(vec![(final_title, body)])
}

fn parse_epub(data: &[u8]) -> Result<Vec<(String, String)>> {
    let mut archive = ZipArchive::new(Cursor::new(data)).context("EPUB không phải ZIP hợp lệ")?;
    anyhow::ensure!(archive.len() <= MAX_EPUB_ENTRIES, "EPUB có quá nhiều mục.");
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let f = archive.by_index(i)?;
        expanded = expanded.saturating_add(f.size());
        anyhow::ensure!(expanded <= MAX_EPUB_EXPANDED, "EPUB giải nén vượt giới hạn 200 MB.");
        anyhow::ensure!(safe_epub_name(f.name()), "EPUB có đường dẫn không an toàn.");
    }
    let container = {
        let mut f = archive.by_name("META-INF/container.xml").context("EPUB thiếu META-INF/container.xml")?;
        let mut s = String::new(); f.read_to_string(&mut s)?; s
    };
    let path_re = Regex::new(r#"full-path=["']([^"']+)["']"#)?;
    let opf_path = path_re.captures(&container).and_then(|c| c.get(1)).map(|m| m.as_str().to_owned())
        .context("Không tìm thấy package OPF trong EPUB.")?;
    anyhow::ensure!(safe_epub_name(&opf_path), "Đường dẫn OPF không an toàn.");
    let opf = {
        let mut f = archive.by_name(&opf_path).context("Không mở được package OPF")?;
        let mut s = String::new(); f.read_to_string(&mut s)?; s
    };
    let base = std::path::Path::new(&opf_path).parent().unwrap_or(std::path::Path::new(""));
    let mut reader = Reader::from_str(&opf);
    let mut buf = Vec::new();
    let mut manifest: HashMap<String, (String, String, String)> = HashMap::new();
    let mut spine: Vec<(String, bool)> = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Empty(e) | Event::Start(e) => {
                match e.local_name().as_ref() {
                    b"item" => {
                        if let (Some(id), Some(href)) = (xml_attr(&e, b"id"), xml_attr(&e, b"href")) {
                            let props = xml_attr(&e, b"properties").unwrap_or_default();
                            let media = xml_attr(&e, b"media-type").unwrap_or_default();
                            manifest.insert(id, (href, props, media));
                        }
                    }
                    b"itemref" => {
                        if let Some(idref) = xml_attr(&e, b"idref") {
                            let linear = xml_attr(&e, b"linear").map(|x| x != "no").unwrap_or(true);
                            spine.push((idref, linear));
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let mut chapters = Vec::new();
    for (id, linear) in spine {
        if !linear { continue; }
        let Some((href, props, media)) = manifest.get(&id) else { continue; };
        if props.split_whitespace().any(|p| p == "nav") || media != "application/xhtml+xml" { continue; }
        let lower = href.to_ascii_lowercase();
        if ["toc", "mulu", "contents"].iter().any(|needle| lower.contains(needle)) { continue; }
        let full = base.join(href);
        let relative = full.to_string_lossy().replace('\\', "/");
        if !safe_epub_name(&relative) { continue; }
        let content = match archive.by_name(&relative) {
            Ok(mut f) => { let mut s=String::new(); f.read_to_string(&mut s)?; s },
            Err(_) => continue,
        };
        for (title, body) in xml_text(&content)? {
            if !title.to_ascii_lowercase().contains("mục lục") && !body.is_empty() {
                chapters.push((title, body));
            }
        }
    }
    anyhow::ensure!(!chapters.is_empty(), "Không tìm thấy nội dung trong EPUB spine.");
    Ok(chapters)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('"', "&quot;").replace('\'', "&#39;")
}

pub fn export_txt(book: &Value, chapters: &[Value]) -> Vec<u8> {
    let title = book.get("title").and_then(Value::as_str).unwrap_or("Novel");
    let done = chapters.iter().filter(|c| c["status"] == "done").count();
    let mut out = format!("{title}\nĐã dịch {done}/{} chương.\nChỉ chương hoàn tất có nội dung; chương khác được ghi chú.\n", chapters.len());
    for c in chapters {
        let title = if c["status"] == "done" { c["translated_title"].as_str().filter(|s| !s.is_empty()).unwrap_or(c["title"].as_str().unwrap_or("Chương")) } else { c["title"].as_str().unwrap_or("Chương") };
        out.push_str("\n\n"); out.push_str(title); out.push_str("\n\n");
        if c["status"] == "done" {
            if let Some(lines) = c["translated"].as_array() {
                out.push_str(&lines.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n\n"));
            }
        } else {
            out.push_str(&format!("[Chưa dịch hoàn tất] {title} ({})", c["status"].as_str().unwrap_or("pending")));
        }
    }
    let mut bytes = vec![0xEF, 0xBB, 0xBF]; bytes.extend_from_slice(out.as_bytes()); bytes
}

pub fn export_epub(book: &Value, chapters: &[Value]) -> Result<Vec<u8>> {
    let title = book["title"].as_str().unwrap_or("Novel");
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let stored = FileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = FileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("mimetype", stored)?; zip.write_all(b"application/epub+zip")?;
        zip.start_file("META-INF/container.xml", deflated)?;
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#)?;
        zip.start_file("OEBPS/style.css", deflated)?;
        zip.write_all(b"body{font-family:serif;line-height:1.8;margin:5%;}p{text-indent:1em;}h1{line-height:1.4}.note{font-style:italic}")?;
        let mut manifest = String::new();
        let mut nav = String::new();
        for (i, c) in chapters.iter().enumerate() {
            let n = i + 1;
            let done = c["status"] == "done";
            let chapter_title = if done { c["translated_title"].as_str().filter(|s| !s.is_empty()).unwrap_or(c["title"].as_str().unwrap_or("Chương")) } else { c["title"].as_str().unwrap_or("Chương chưa dịch") };
            let mut xhtml = format!(r#"<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" lang="vi"><head><title>{}</title><link rel="stylesheet" type="text/css" href="style.css"/></head><body><h1>{}</h1>"#, html_escape(chapter_title), html_escape(chapter_title));
            if done {
                if let Some(lines) = c["translated"].as_array() {
                    for line in lines.iter().filter_map(Value::as_str) {
                        xhtml.push_str("<p>"); xhtml.push_str(&html_escape(line)); xhtml.push_str("</p>");
                    }
                }
            } else {
                xhtml.push_str("<p class=\"note\">Chương chưa dịch hoàn tất; không xuất bản dịch dở.</p>");
            }
            xhtml.push_str("</body></html>");
            let path = format!("OEBPS/chapter_{n:04}.xhtml");
            zip.start_file(&path, deflated)?; zip.write_all(xhtml.as_bytes())?;
            manifest.push_str(&format!(r#"<item id="c{n}" href="chapter_{n:04}.xhtml" media-type="application/xhtml+xml"/>"#));
            nav.push_str(&format!(r#"<li><a href="chapter_{n:04}.xhtml">{}</a></li>"#, html_escape(chapter_title)));
        }
        zip.start_file("OEBPS/nav.xhtml", deflated)?;
        zip.write_all(format!(r#"<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Mục lục</title></head><body><nav epub:type="toc"><h1>Mục lục</h1><ol>{nav}</ol></nav></body></html>"#).as_bytes())?;
        zip.start_file("OEBPS/content.opf", deflated)?;
        zip.write_all(format!(r#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="bookid">novelreader-{}</dc:identifier><dc:title>{}</dc:title><dc:language>vi</dc:language><dc:creator>NovelReader</dc:creator></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="style.css" media-type="text/css"/>{manifest}</manifest><spine><itemref idref="nav"/>{}</spine></package>"#,
            book["id"].as_i64().unwrap_or(1), html_escape(title),
            (1..=chapters.len()).map(|n| format!(r#"<itemref idref="c{n}"/>"#)).collect::<String>()
        ).as_bytes())?;
        zip.finish()?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_txt_and_epub() {
        let book=serde_json::json!({"id":1,"title":"测试"});
        let chapters=vec![serde_json::json!({"title":"第1章","translated_title":"Chương 1","status":"done","translated":["Xin chào <bạn>"]})];
        let txt=export_txt(&book,&chapters);
        assert!(String::from_utf8_lossy(&txt).contains("Xin chào <bạn>"));
        let epub=export_epub(&book,&chapters).unwrap();
        let mut zip=ZipArchive::new(Cursor::new(epub)).unwrap();
        assert!(zip.by_name("mimetype").is_ok());
        let mut x=String::new();zip.by_name("OEBPS/chapter_0001.xhtml").unwrap().read_to_string(&mut x).unwrap();
        assert!(x.contains("Xin chào &lt;bạn&gt;"));
    }
}
