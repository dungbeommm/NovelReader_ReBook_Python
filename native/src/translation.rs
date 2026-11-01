use anyhow::{bail,Result};
use serde::Serialize;

#[derive(Debug,Clone,Copy,Serialize)]
pub struct ModelSpec{
 pub key:&'static str,pub model_repo:&'static str,pub ct2_repo:&'static str,pub subdir:&'static str,
 pub max_input:usize,pub max_output:usize,pub default_beam:usize,pub repetition_penalty:f32,
 pub opener_spacing:bool,pub newline_token:Option<&'static str>,
}
pub const MODELS:&[ModelSpec]=&[
 ModelSpec{key:"HachimiMT-60",model_repo:"ngocdang83/HachimiMT-60-zh-vi",ct2_repo:"ngocdang83/HachimiMT-60-zh-vi",subdir:"ct2-int8_float32",max_input:160,max_output:300,default_beam:2,repetition_penalty:1.2,opener_spacing:false,newline_token:None},
 ModelSpec{key:"HachimiMT-60-QT",model_repo:"ngocdang83/HachimiMT-60-QT",ct2_repo:"ngocdang83/HachimiMT-60-QT",subdir:"ct2-int8_float32",max_input:160,max_output:300,default_beam:2,repetition_penalty:1.2,opener_spacing:false,newline_token:None},
 ModelSpec{key:"HachimiMT-40-QT",model_repo:"ngocdang83/HachimiMT-40-QT",ct2_repo:"ngocdang83/HachimiMT-40-QT",subdir:"ct2-int8_float32",max_input:160,max_output:256,default_beam:1,repetition_penalty:1.05,opener_spacing:true,newline_token:Some("<NL>")},
 ModelSpec{key:"HachimiMT-30",model_repo:"ngocdang83/HachimiMT-30-zh-vi",ct2_repo:"ngocdang83/HachimiMT-30-zh-vi",subdir:"ct2-int8_float32",max_input:160,max_output:512,default_beam:1,repetition_penalty:1.0,opener_spacing:false,newline_token:None},
 ModelSpec{key:"MoxhiMT-60",model_repo:"DanVP/MoxhiMT-60",ct2_repo:"DanVP/MoxhiMT-60",subdir:"ct2-int8",max_input:160,max_output:300,default_beam:2,repetition_penalty:1.2,opener_spacing:false,newline_token:None},
 ModelSpec{key:"MoxhiMT-30",model_repo:"DanVP/MoxhiMT-30",ct2_repo:"DanVP/MoxhiMT-30",subdir:"ct2-int8_float32",max_input:160,max_output:512,default_beam:2,repetition_penalty:1.2,opener_spacing:false,newline_token:None},
 ModelSpec{key:"MoxhiMT-30-QT",model_repo:"DanVP/MoxhiMT-30-QT",ct2_repo:"DanVP/MoxhiMT-30-QT",subdir:"ct2-int8_float32",max_input:160,max_output:512,default_beam:1,repetition_penalty:1.2,opener_spacing:true,newline_token:None},
 ModelSpec{key:"HirashibaMT-Medium",model_repo:"Moleys/hirashiba-mt-medium",ct2_repo:"ngungodan/hirashiba-mt-medium-ct2",subdir:"ct2-int8_float32",max_input:128,max_output:256,default_beam:4,repetition_penalty:1.0,opener_spacing:false,newline_token:None},
 ModelSpec{key:"HirashibaMT-Tiny",model_repo:"chi-vi/hirashiba-mt-tiny-zh-vi",ct2_repo:"ngungodan/hirashiba-mt-tiny-zh-vi-ct2",subdir:"ct2-int8-keeppad",max_input:160,max_output:512,default_beam:1,repetition_penalty:1.0,opener_spacing:false,newline_token:None},
];
pub fn model(key:&str)->Result<&'static ModelSpec>{MODELS.iter().find(|m|m.key==key).ok_or_else(||anyhow::anyhow!("Model không tồn tại: {key}"))}

/// Token-aware splitter. It asks the native SentencePiece worker for counts,
/// binary-searches the largest fitting Unicode prefix, then backs up to punctuation.
pub fn split_for_ct2<F>(text:&str,max_tokens:usize,terms:&[String],mut token_count:F)->Result<Vec<String>>
where F:FnMut(&str)->Result<usize>{
 if text.trim().is_empty(){return Ok(vec![]);}
 anyhow::ensure!(max_tokens>=2,"Token cap phải từ 2 trở lên.");
 let chars:Vec<char>=text.chars().collect();let mut start=0usize;let mut chunks=Vec::new();
 while start<chars.len(){
  let rest:String=chars[start..].iter().collect();
  if token_count(&rest)?<=max_tokens{chunks.push(rest);break;}
  let mut low=1usize;let mut high=chars.len()-start;let mut best=0usize;
  while low<=high{
   let mid=low+(high-low)/2;let candidate:String=chars[start..start+mid].iter().collect();
   if token_count(&candidate)?<=max_tokens{best=mid;low=mid+1;}else{high=mid.saturating_sub(1);}
  }
  if best==0{bail!("Một ký tự đơn vượt token cap; không thể chia an toàn.");}
  // Prefer the latest sentence boundary in the final 40% of the fitting window.
  let floor=best.saturating_mul(6)/10;
  let boundary=(floor.max(1)..best).rev().find(|i|matches!(chars[start+*i-1],'。'|'！'|'？'|'!'|'?'|'；'|';'|'…'));
  if let Some(b)=boundary{best=b;}
  // Never split a known glossary term across chunks.
  for term in terms{
   if term.is_empty(){continue;}
   let term_chars:Vec<char>=term.chars().collect();
   if term_chars.len()>best{continue;}
   let lo=best.saturating_sub(term_chars.len()-1);
   for pos in lo..=best{
    if pos+term_chars.len()<=chars.len()-start{
     let candidate=&chars[start+pos..start+pos+term_chars.len()];
     if candidate==term_chars.as_slice()&&pos<best&&best<pos+term_chars.len(){
      if pos>0{best=pos;}else{best=pos+term_chars.len();}break;
     }
    }
   }
  }
  let piece:String=chars[start..start+best].iter().collect();
  if piece.is_empty(){bail!("Chunker không tiến được; hủy để tránh mất dữ liệu.");}
  chunks.push(piece);start+=best;
 }
 let joined=chunks.concat();
 anyhow::ensure!(joined==text,"Chunker làm thay đổi hoặc mất nội dung nguồn.");
 Ok(chunks)
}

pub fn assemble(chunks:&[String],translated:&[String])->Result<String>{
 anyhow::ensure!(chunks.len()==translated.len(),"Số chunk dịch không khớp nguồn.");
 Ok(translated.iter().map(|s|s.trim()).filter(|s|!s.is_empty()).collect::<Vec<_>>().join(" "))
}
pub fn postprocess(input:&str)->String{
 let mut text=input.replace('。',".").replace('，',",").replace('！',"!").replace('？',"?").replace('；',";").replace('：',":");
 let re=regex::Regex::new(r"\s+([,.!?;:])").expect("static regex");
 text=re.replace_all(&text,"$1").into_owned();
 let mut chars=text.chars();match chars.next(){Some(first)=>first.to_uppercase().chain(chars).collect(),None=>String::new()}
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn splits_without_losing_unicode(){
  let source="第一句。第二句！第三句？";
  let chunks=split_for_ct2(source,5,&[],|s|Ok(s.chars().count())).unwrap();
  assert_eq!(chunks.concat(),source);assert!(chunks.len()>1);
 }
 #[test]fn glossary_term_not_split(){
  let source="abc HachimiMT xyz";
  let chunks=split_for_ct2(source,8,&["HachimiMT".into()],|s|Ok(s.chars().count())).unwrap();
  assert_eq!(chunks.concat(),source);
  assert!(!chunks.iter().any(|c|c=="HachimiM"));
 }
 #[test]fn postprocess_punctuation(){assert_eq!(postprocess("你好， 世界。"),"你好, 世界.");}
}
