use anyhow::{bail, Result};
use chardetng::EncodingDetector;
use regex::Regex;

pub const MAX_FILE: usize = 50 * 1024 * 1024;

pub fn decode_txt(bytes: &[u8]) -> Result<(String, String)> {
    if bytes.len() > MAX_FILE {
        bail!("File tối đa 50 MB.");
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Ok((String::from_utf8(bytes[3..].to_vec())?, "utf-8-sig".into()));
    }
    if let Ok(value) = std::str::from_utf8(bytes) {
        return Ok((value.to_owned(), "utf-8".into()));
    }
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);
    let (value, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        bail!("Không thể giải mã tệp; hãy chuyển tệp sang UTF-8.");
    }
    Ok((value.into_owned(), encoding.name().into()))
}

pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{feff}', "")
        .replace('“', "\"")
        .replace('”', "\"")
        .replace('\u{3000}', " ")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn paragraphs(text: &str) -> Vec<String> {
    normalize(text).lines().map(ToOwned::to_owned).collect()
}

fn looks_like_toc(title: &str, body: &str, heading: &Regex) -> bool {
    let lower = title.trim().to_lowercase();
    if ["目录", "目錄", "mục lục", "table of contents", "contents", "toc"]
        .iter().any(|term| lower == *term || lower.contains("mục lục")) {
        return true;
    }
    let lines: Vec<&str> = body.lines().map(str::trim).filter(|s| !s.is_empty()).collect();
    if lines.len() < 3 {
        return false;
    }
    let similar = lines.iter().filter(|line| {
        heading.is_match(line) || line.starts_with("Chương ") ||
            line.chars().next().is_some_and(|ch| ch.is_ascii_digit())
    }).count();
    similar * 10 >= lines.len() * 4
}

pub fn split_chapters(text: &str) -> Result<Vec<(String, String)>> {
    let lines = paragraphs(text);
    if lines.is_empty() {
        bail!("Tệp không có nội dung đọc được.");
    }
    let title_re = Regex::new(r"^(?:第[零〇一二三四五六七八九十百千万两兩億亿0-9]+[章节節回卷].*|(?i:Chapter)\s+\d+\b.*)$")?;
    let mut headings = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if title_re.is_match(line) {
            headings.push(i);
        }
    }
    let mut result = Vec::new();
    if !headings.is_empty() {
        if headings[0] != 0 {
            let pre = lines[..headings[0]].join("\n");
            if !looks_like_toc("Lời mở đầu", &pre, &title_re) {
                result.push(("Lời mở đầu".into(), pre));
            }
        }
        for (index, &start) in headings.iter().enumerate() {
            let end = headings.get(index + 1).copied().unwrap_or(lines.len());
            let title = lines[start].clone();
            let body = lines[start + 1..end].join("\n");
            if !looks_like_toc(&title, &body, &title_re) {
                result.push((title, body));
            }
        }
    } else {
        let mut part = 1;
        let joined = lines.join("\n");
        let chars: Vec<char> = joined.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let mut end = (start + 3000).min(chars.len());
            if end < chars.len() {
                if let Some(position) = (start..end).rev().find(|&i| chars[i] == '\n') {
                    if position > start + 1500 {
                        end = position;
                    }
                }
            }
            if end <= start { end = (start + 3000).min(chars.len()); }
            let slice: String = chars[start..end].iter().collect();
            if !slice.trim().is_empty() {
                result.push((format!("Phần {part}"), slice.trim().to_owned()));
                part += 1;
            }
            start = end;
            while start < chars.len() && chars[start] == '\n' { start += 1; }
        }
    }
    if result.is_empty() {
        bail!("Không tìm thấy chương hợp lệ.");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_titles_and_body() {
        let input = "第1章 初 gặp\n你好。\n第2章 Trở về\n再见。";
        let chapters = split_chapters(input).unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[1].1, "再见。");
    }
    #[test]
    fn preserves_unicode() {
        assert_eq!(paragraphs(" 你好。\r\n\r\n Thế giới。"), vec!["你好。", "Thế giới。"]);
    }
    #[test]
    fn empty_rejected() {
        assert!(split_chapters(" \r\n ").is_err());
    }
}
