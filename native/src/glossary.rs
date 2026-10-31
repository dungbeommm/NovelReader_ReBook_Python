use anyhow::{bail,Result};
use serde_json::Value;
use std::{collections::HashMap,fs,path::Path};
use unicode_normalization::UnicodeNormalization;

pub const HEADERS:[&str;5]=["source_zh","target_vi","type","aliases_vi","enabled"];
pub const MAX_BYTES:usize=2_000_000;
pub const MAX_ROWS:usize=5_000;
pub const MARKERS:[&str;6]=["QX7","KX7","RX7","ZX9","ZQ1","VX1"];

#[derive(Debug,Clone,PartialEq,Eq,Hash)]
pub struct GlossaryEntry{pub source_zh:String,pub target_vi:String,pub entry_type:String,pub aliases_vi:Vec<String>}
#[derive(Debug,Clone)]
pub struct Binding{pub marker:String,pub entry:GlossaryEntry,pub occurrences:usize}
#[derive(Debug,Clone)]
pub struct Protection{pub original:String,pub protected:String,pub bindings:Vec<Binding>,pub source_hits:usize,pub skipped:usize}

fn clean(value:&Value)->String{
 match value{
  Value::Null=>String::new(),Value::String(s)=>s.trim().nfc().collect(),
  Value::Bool(v)=>v.to_string(),Value::Number(n)=>n.to_string(),_=>String::new()
 }
}
fn enabled(value:&Value)->Result<bool>{
 if value.is_null()||clean(value).is_empty(){return Ok(true);}
 match value{
  Value::Bool(v)=>Ok(*v),Value::Number(n)=>Ok(n.as_f64().unwrap_or(0.0)!=0.0),
  _=>match clean(value).to_lowercase().as_str(){
   "1"|"true"|"yes"|"y"|"on"|"x"|"✓"|"bật"=>Ok(true),
   "0"|"false"|"no"|"n"|"off"|"✗"|"tắt"=>Ok(false),
   _=>bail!("Giá trị enabled không hợp lệ: {}. Dùng true/false hoặc 1/0.",clean(value))
  }
 }
}
fn row_values(raw:&Value)->Result<[Value;5]>{
 if let Some(obj)=raw.as_object(){
  return Ok([
   obj.get("source_zh").or_else(||obj.get("source")).cloned().unwrap_or(Value::Null),
   obj.get("target_vi").or_else(||obj.get("target")).cloned().unwrap_or(Value::Null),
   obj.get("type").or_else(||obj.get("entry_type")).cloned().unwrap_or(Value::Null),
   obj.get("aliases_vi").or_else(||obj.get("aliases")).cloned().unwrap_or(Value::Null),
   obj.get("enabled").cloned().unwrap_or(Value::Bool(true)),
  ]);
 }
 if let Some(array)=raw.as_array(){
  let mut values=array.iter().take(5).cloned().collect::<Vec<_>>();
  values.resize(5,Value::Null);
  if array.len()<5{values[4]=Value::Bool(true);}
  return Ok(values.try_into().unwrap());
 }
 bail!("Mỗi dòng glossary phải là object hoặc array.")
}
pub fn compile(value:&Value)->Result<Vec<GlossaryEntry>>{
 let rows=if let Some(obj)=value.as_object(){obj.get("data").or_else(||obj.get("entries")).unwrap_or(value)}else{value};
 let rows=if let Some(s)=rows.as_str(){
  if let Ok(v)=serde_json::from_str::<Value>(s){v}else{
   let mut reader=csv::ReaderBuilder::new().delimiter(b'\t').has_headers(true).from_reader(s.as_bytes());
   let headers=reader.headers()?.clone();
   let mut out=Vec::new();for record in reader.records(){let rec=record?;let mut map=serde_json::Map::new();for (i,h) in headers.iter().enumerate(){map.insert(h.into(),Value::String(rec.get(i).unwrap_or("").into()));}out.push(Value::Object(map));}
   Value::Array(out)
  }
 }else{rows.clone()};
 let array=rows.as_array().context("Glossary phải là bảng hoặc danh sách các dòng.")?;
 anyhow::ensure!(array.len()<=MAX_ROWS,"Glossary tối đa {MAX_ROWS} dòng.");
 let mut merged:HashMap<String,GlossaryEntry>=HashMap::new();
 let mut first_row:HashMap<String,usize>=HashMap::new();
 for (index,raw) in array.iter().enumerate(){
  let values=row_values(raw)?;if !enabled(&values[4])?{continue;}
  let source=clean(&values[0]);let target=clean(&values[1]);let kind=clean(&values[2]);
  anyhow::ensure!(!source.is_empty()&&!target.is_empty(),"Dòng glossary {}: source_zh và target_vi là bắt buộc.",index+1);
  let aliases_raw=match &values[3]{Value::Array(a)=>a.iter().map(clean).collect::<Vec<_>>().join("|"),v=>clean(v)};
  let mut aliases=aliases_raw.split('|').map(|s|s.trim().nfc().collect::<String>())
   .filter(|s|!s.is_empty()&&s!=&target).collect::<Vec<_>>();
  aliases.sort_by(|a,b|b.chars().count().cmp(&a.chars().count()).then_with(||a.cmp(b)));aliases.dedup();
  if let Some(existing)=merged.get_mut(&source){
   anyhow::ensure!(existing.target_vi==target,"Xung đột source_zh {source:?}: dòng {} và {} có target khác nhau.",first_row[&source],index+1);
   existing.aliases_vi.extend(aliases);existing.aliases_vi.sort_by(|a,b|b.chars().count().cmp(&a.chars().count()).then_with(||a.cmp(b)));existing.aliases_vi.dedup();
   if existing.entry_type.is_empty(){existing.entry_type=kind;}
  }else{
   first_row.insert(source.clone(),index+1);
   merged.insert(source.clone(),GlossaryEntry{source_zh:source,target_vi:target,entry_type:kind,aliases_vi:aliases});
  }
 }
 let mut result=merged.into_values().collect::<Vec<_>>();
 result.sort_by(|a,b|b.source_zh.chars().count().cmp(&a.source_zh.chars().count()).then_with(||a.source_zh.cmp(&b.source_zh)));
 Ok(result)
}
fn matches(source:&str,entries:&[GlossaryEntry])->Vec<(usize,usize,usize)>{
 let mut candidates=Vec::new();
 for (index,entry) in entries.iter().enumerate(){
  let mut start=0;
  while let Some(found)=source[start..].find(&entry.source_zh){
   let s=start+found;let e=s+entry.source_zh.len();candidates.push((s,e,index));start=s+entry.source_zh.len();
  }
 }
 candidates.sort_by(|a,b|a.0.cmp(&b.0).then_with(||(b.1-b.0).cmp(&(a.1-a.0))).then_with(||a.2.cmp(&b.2)));
 let mut out=Vec::new();let mut occupied=0;
 for (s,e,i) in candidates{if s<occupied{continue;}out.push((s,e,i));occupied=e;}
 out
}
pub fn source_terms(entries:&[GlossaryEntry])->Vec<String>{entries.iter().map(|e|e.source_zh.clone()).collect()}
pub fn protect_line(text:&str,entries:&[GlossaryEntry])->Protection{
 let spans=matches(text,entries);let source_hits=spans.len();
 if spans.is_empty(){return Protection{original:text.into(),protected:text.into(),bindings:vec![],source_hits,skipped:0};}
 let mut ordered=Vec::new();for (_,_,i) in &spans{if !ordered.contains(i){ordered.push(*i);}}
 let mut marker_pool=MARKERS.iter().filter(|m|{
  !text.to_lowercase().contains(&m.to_lowercase())&&!entries.iter().any(|e|e.target_vi.to_lowercase().contains(&m.to_lowercase())||e.aliases_vi.iter().any(|a|a.to_lowercase().contains(&m.to_lowercase())))
 }).map(|s|s.to_string()).collect::<Vec<_>>();
 let mut assignments=HashMap::new();for (i,entry) in ordered.iter().enumerate(){if i<marker_pool.len(){assignments.insert(*entry,marker_pool[i].clone());}}
 let mut protected=text.to_string();let mut counts:HashMap<usize,usize>=HashMap::new();
 for (start,end,index) in spans.iter().rev(){if let Some(marker)=assignments.get(index){protected.replace_range(*start..*end,marker);*counts.entry(*index).or_default()+=1;}}
 let mut bindings=Vec::new();for index in ordered{if let (Some(marker),Some(count))=(assignments.get(&index),counts.get(&index)){
  bindings.push(Binding{marker:marker.clone(),entry:entries[index].clone(),occurrences:*count});
 }}
 let protected_occurrences:usize=bindings.iter().map(|b|b.occurrences).sum();
 Protection{original:text.into(),protected,bindings,source_hits,skipped:source_hits-protected_occurrences}
}
fn marker_count(text:&str,marker:&str)->usize{
 let bytes=text.as_bytes();let needle=marker.as_bytes();let mut count=0;let mut offset=0;
 while offset+needle.len()<=bytes.len(){
  if let Some(pos)=text[offset..].to_ascii_lowercase().find(&marker.to_ascii_lowercase()){
   let start=offset+pos;let end=start+needle.len();
   let left=if start==0{None}else{text[..start].chars().next_back()};
   let right=text[end..].chars().next();
   let is_word=|c:char|c.is_ascii_alphanumeric();
   if !left.is_some_and(is_word)&&!right.is_some_and(is_word){count+=1;}
   offset=end;
  }else{break;}
 }
 count
}
pub fn restore(protection:&Protection,translated:&str)->Result<String>{
 let mut out=translated.to_string();
 for binding in &protection.bindings{
  let count=marker_count(&out,&binding.marker);
  anyhow::ensure!(count==binding.occurrences,"Glossary placeholder {} bị mất hoặc lặp (mong đợi {}, nhận {}).",binding.marker,binding.occurrences,count);
  let re=regex::RegexBuilder::new(&regex::escape(&binding.marker)).case_insensitive(true).build()?;
  out=re.replace_all(&out,regex::NoExpand(&binding.entry.target_vi)).into_owned();
 }
 Ok(out)
}
fn replace_literal(text:&str,needle:&str,replacement:&str)->(String,usize){
 if needle.is_empty(){return (text.into(),0);}
 let pattern=regex::RegexBuilder::new(&regex::escape(needle)).case_insensitive(false).build();
 let Ok(re)=pattern else{return (text.into(),0);};
 let count=re.find_iter(text).count();
 (re.replace_all(text,regex::NoExpand(replacement)).into_owned(),count)
}
pub fn apply_aliases(source:&str,translated:&str,entries:&[GlossaryEntry])->(String,Value){
 let matched=matches(source,entries);let mut unique=Vec::new();for (_,_,i) in matched{if !unique.contains(&i){unique.push(i);}}
 let mut out=translated.to_string();let mut replacements=0;let mut satisfied=0;let mut unresolved=0;let mut terms=Vec::new();
 for index in unique{
  let entry=&entries[index];let mut row_replacements=0;
  for alias in &entry.aliases_vi{let (next,count)=replace_literal(&out,alias,&entry.target_vi);out=next;row_replacements+=count;}
  if row_replacements>0{replacements+=row_replacements;}
  else if out.contains(&entry.target_vi){satisfied+=1;}
  else{unresolved+=1;terms.push(entry.source_zh.clone());}
 }
 (out,serde_json::json!({"entries":entries.len(),"source_hits":matched.len(),"replacements":replacements,"satisfied":satisfied,"unresolved":unresolved,"unresolved_terms":terms}))
}
pub fn read_file(path:&Path)->Result<Value>{
 anyhow::ensure!(path.metadata()?.len()<=MAX_BYTES as u64,"File glossary vượt 2 MB.");
 match path.extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_lowercase().as_str(){
  "json"=>{let data=std::fs::read(path)?;let value:Value=serde_json::from_slice(&data)?;Ok(value.get("entries").cloned().unwrap_or(value))}
  "tsv"=>{let mut reader=csv::ReaderBuilder::new().delimiter(b'\t').from_path(path)?;let headers=reader.headers()?.clone();let mut rows=Vec::new();for rec in reader.records(){let rec=rec?;let mut obj=serde_json::Map::new();for (i,h) in headers.iter().enumerate(){obj.insert(h.into(),Value::String(rec.get(i).unwrap_or("").into()));}rows.push(Value::Object(obj));}Ok(Value::Array(rows))}
  _=>bail!("Chỉ hỗ trợ glossary .tsv hoặc .json.")
 }
}
pub fn write_file(path:&Path,value:&Value,file_format:&str)->Result<()>{
 let entries=compile(value)?;
 if file_format=="json"{let rows=entries.iter().map(|e|serde_json::json!({"source_zh":e.source_zh,"target_vi":e.target_vi,"type":e.entry_type,"aliases_vi":e.aliases_vi.join("|"),"enabled":true})).collect::<Vec<_>>();
  std::fs::write(path,serde_json::to_vec_pretty(&rows)?)?;return Ok(());
 }
 anyhow::ensure!(file_format=="tsv","Định dạng xuất glossary phải là tsv hoặc json.");
 let mut writer=csv::WriterBuilder::new().delimiter(b'\t').from_path(path)?;
 writer.write_record(HEADERS)?;
 for e in entries{writer.write_record([e.source_zh,e.target_vi,e.entry_type,e.aliases_vi.join("|"),"true".into()])?;}
 writer.flush()?;Ok(())
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn longest_match_and_restore(){
  let rows=serde_json::json!([["张三","Trương Tam","name","Trương Ba",true],["张","Trương", "name","",true]]);
  let entries=compile(&rows).unwrap();let p=protect_line("张三来了",&entries);
  assert_eq!(p.protected,"QX7来了");assert_eq!(restore(&p,"QX7 đã đến").unwrap(),"Trương Tam đã đến");
 }
 #[test]fn conflict_is_rejected(){
  let rows=serde_json::json!([["张三","Trương Tam","","",true],["张三","Trương Ba","","",true]]);
  assert!(compile(&rows).is_err());
 }
 #[test]fn alias_canonicalization(){
  let rows=serde_json::json!([["张三","Trương Tam","","Trương Ba",true]]);
  let entries=compile(&rows).unwrap();let (text,_)=apply_aliases("张三来了","Trương Ba đến",&entries);
  assert_eq!(text,"Trương Tam đến");
 }
}
