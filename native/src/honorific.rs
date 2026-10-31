use serde_json::{json,Value};
use std::collections::HashMap;

const KINSHIP:&[(&str,&str,&[&str])]=&[
 ("父亲","phụ thân",&["cha","bố","phụ thân"]),("父亲大人","phụ thân",&["cha","bố"]),
 ("母亲","mẫu thân",&["mẹ","má","mẫu thân"]),("母亲大人","mẫu thân",&["mẹ","má"]),
 ("父母","phụ mẫu",&["cha mẹ","song thân"]),("双亲","song thân",&["cha mẹ"]),
 ("爷爷","gia gia",&["ông nội"]),("奶奶","nãi nãi",&["bà nội"]),
 ("祖父","tổ phụ",&["ông nội"]),("祖母","tổ mẫu",&["bà nội"]),
 ("外公","ngoại công",&["ông ngoại"]),("外婆","ngoại bà",&["bà ngoại"]),
 ("师尊","sư tôn",&["sư phụ","thầy"]),("师父","sư phụ",&["sư phó","thầy"]),
 ("师傅","sư phụ",&["sư phó","thầy"]),("师娘","sư nương",&["sư mẫu"]),
 ("大师兄","đại sư huynh",&["đại sư ca"]),("大师姐","đại sư tỷ",&["đại sư tỷ"]),
 ("大师弟","đại sư đệ",&[]),("大师妹","đại sư muội",&[]),
 ("小师兄","tiểu sư huynh",&["sư huynh"]),("小师姐","tiểu sư tỷ",&["sư tỷ"]),
 ("小师弟","tiểu sư đệ",&["sư đệ"]),("小师妹","tiểu sư muội",&["sư muội"]),
 ("师兄弟","sư huynh đệ",&["sư huynh弟"]),("师姐妹","sư tỷ muội",&[]),
 ("师兄","sư huynh",&["sư ca"]),("师姐","sư tỷ",&["sư tỷ"]),
 ("师弟","sư đệ",&["sư đệ"]),("师妹","sư muội",&["sư muội"]),
 ("师叔","sư thúc",&["sư chú"]),("师伯","sư bá",&["sư bác"]),
 ("师祖","sư tổ",&["tổ sư"]),("祖师","tổ sư",&["sư tổ"]),
 ("师门","sư môn",&["môn phái"]),("同门","đồng môn",&["người cùng môn"]),
 ("掌门","chưởng môn",&["chưởng môn nhân"]),("门主","môn chủ",&[]),
 ("宗主","tông chủ",&["tông chủ nhân"]),("宗门","tông môn",&["tông phái"]),
 ("长老","trưởng lão",&["trưởng lão"]),("太上长老","thái thượng trưởng lão",&[]),
 ("道侣","đạo lữ",&["bạn đời","đạo lữ"]),("道友","đạo hữu",&["bạn đạo"]),
 ("前辈","tiền bối",&["bậc tiền bối"]),("晚辈","vãn bối",&["hậu bối"]),
 ("公子","công tử",&["thiếu gia"]),("小姐","tiểu thư",&["cô nương"]),
 ("少爷","thiếu gia",&["công tử"]),("夫人","phu nhân",&["bà"]),
 ("娘子","nương tử",&["thê tử","vợ"]),("相公","tướng công",&["phu quân","chồng"]),
 ("夫君","phu quân",&["chồng"]),("妻子","thê tử",&["vợ"]),
 ("兄长","huynh trưởng",&["anh trai"]),("哥哥","ca ca",&["anh trai"]),
 ("姐姐","tỷ tỷ",&["chị gái"]),("弟弟","đệ đệ",&["em trai"]),
 ("妹妹","muội muội",&["em gái"]),("叔父","thúc phụ",&["chú"]),
 ("伯父","bá phụ",&["bác"]),("姑母","cô mẫu",&["cô"]),
 ("姨母","di mẫu",&["dì"]),("舅舅","cữu cữu",&["cậu"]),
];
const PRONOUNS:&[(&str,&str,&[&str])]=&[
 ("本座","bản tọa",&["ta"]),("本尊","bản tôn",&["ta"]),("本王","bản vương",&["ta"]),
 ("本宫","bản cung",&["ta"]),("贫道","bần đạo",&["ta"]),("老夫","lão phu",&["ta"]),
 ("老朽","lão hủ",&["ta"]),("在下","tại hạ",&["ta"]),("小爷","tiểu gia",&["ta"]),
 ("朕","trẫm",&["ta"]),("吾","ngô",&["ta"]),("我","ta",&["tôi","mình"]),
 ("我们","chúng ta",&["chúng tôi","bọn ta"]),("咱们","chúng ta",&["chúng tôi"]),
 ("尔等","nhĩ đẳng",&["các ngươi"]),("汝","nhữ",&["ngươi"]),("你","ngươi",&["bạn","cậu"]),
 ("您","ngài",&["ông","bà","bạn"]),("你们","các ngươi",&["các bạn"]),
 ("他","hắn",&["anh ta","anh ấy"]),("她","nàng",&["cô ấy","chị ấy"]),
 ("它","nó",&["nó"]),("他们","bọn hắn",&["họ","bọn họ"]),
 ("她们","các nàng",&["họ","các cô ấy"]),("吾等","chúng ta",&["chúng tôi"]),
];

fn is_word(c:char)->bool{c.is_alphanumeric()||c=='_'}
fn case_like(original:&str,replacement:&str)->String{
 if original.chars().all(|c|!c.is_alphabetic()||c.is_uppercase()){replacement.to_uppercase()}
 else if original.chars().next().is_some_and(char::is_uppercase){
  let mut c=replacement.chars();c.next().map(|f|f.to_uppercase().collect::<String>()+c.as_str()).unwrap_or_default()
 }else{replacement.to_string()}
}
fn find_spans(text:&str,needle:&str)->Vec<(usize,usize)>{
 if needle.is_empty(){return vec![];}
 let mut out=Vec::new();let mut offset=0;
 while offset<text.len(){
  let Some(pos)=text[offset..].find(needle)else{break;};let start=offset+pos;let end=start+needle.len();
  let left=if start==0{None}else{text[..start].chars().next_back()};
  let right=text[end..].chars().next();
  if !left.is_some_and(is_word)&&!right.is_some_and(is_word){out.push((start,end));}
  offset=end.max(start+1);
 }
 out
}
fn replace_span(text:&str,start:usize,end:usize,replacement:&str)->String{
 let mut out=String::with_capacity(text.len()+replacement.len());out.push_str(&text[..start]);out.push_str(replacement);out.push_str(&text[end..]);out
}
fn classical_context(source:&str)->bool{
 ["第","修仙","修真","宗门","仙尊","魔尊","师尊","徒弟","本座","贫道","妖兽","灵力","元婴","渡劫","长老","掌门","宗主","道友","法宝","仙界","魔界"]
  .iter().any(|term|source.contains(term))
}
fn source_counts(source:&str,table:&[(&str,&str,&[&str])])->HashMap<&'static str,usize>{
 let mut counts=HashMap::new();
 for (source_term,target,_) in table{
  let count=source.matches(source_term).count();if count>0{*counts.entry(*target).or_default()+=count;}
 }
 counts
}
fn protected_spans(text:&str,terms:&[String])->Vec<(usize,usize)>{
 let mut spans=Vec::new();
 for term in terms{spans.extend(find_spans(text,term));}
 spans.sort_by(|a,b|a.0.cmp(&b.0).then_with(||(b.1-b.0).cmp(&(a.1-a.0))));
 let mut out=Vec::new();let mut end=0;
 for (s,e) in spans{if s>=end{out.push((s,e));end=e;}}
 out
}
fn rewrite_table(text:&str,source:&str,table:&[(&str,&str,&[&str])],mode:&str)->(String,usize){
 let counts=source_counts(source,table);
 let mut out=text.to_string();let mut replacements=0;
 let mut canonical=HashMap::<&str,usize>::new();
 for (target,count) in &counts{canonical.insert(target,*count);}
 let mut protected_terms=table.iter().map(|(_,target,aliases)|{
  let mut v=vec![target.to_string()];v.extend(aliases.iter().map(|s|s.to_string()));v
 }).flatten().collect::<Vec<_>>();
 protected_terms.sort_by_key(|s|std::cmp::Reverse(s.chars().count()));
 let protected=protected_spans(&out,&protected_terms);
 let mut drift:Vec<(String,String,usize)>=Vec::new();
 for (source_term,target,aliases) in table{
  let budget=canonical.get(target).copied().unwrap_or(0);if budget==0{continue;}
  for alias in *aliases{
   if alias==target{continue;}
   drift.push((alias.to_string(),target.to_string(),budget));
  }
 }
 drift.sort_by_key(|(alias,_,_)|std::cmp::Reverse(alias.chars().count()));
 for (alias,target,budget) in drift{
  if budget==0{continue;}
  let spans=find_spans(&out,&alias);
  let mut used=0;
  for (start,end) in spans.into_iter().rev(){
   if used>=budget{break;}
   if protected.iter().any(|(s,e)|start<*e&&end>*s){continue;}
   let original=out[start..end].to_string();
   let replacement=case_like(&original,&target);
   out=replace_span(&out,start,end,&replacement);used+=1;replacements+=1;
  }
 }
 if mode=="off"{(text.to_string(),0)}else{(out,replacements)}
}
pub fn normalize(source:&str,translated:&str,mode:&str,apply_kinship:bool,apply_pronouns:bool,kinship_mode:&str)->(String,Value){
 if mode=="off"||(!apply_kinship&&!apply_pronouns){return(translated.into(),json!({"enabled":false,"replacements":0}));}
 let classical=classical_context(source);
 if mode=="safe"&&!classical{return(translated.into(),json!({"enabled":true,"skipped":"non_classical_context","replacements":0}));}
 let mut output=translated.to_string();let mut kinship_count=0;let mut pronoun_count=0;
 if apply_kinship{
  let table=if kinship_mode=="hanviet"{KINSHIP}else{KINSHIP};
  let (next,count)=rewrite_table(&output,source,table,mode);output=next;kinship_count=count;
 }
 if apply_pronouns{
  let (next,count)=rewrite_table(&output,source,PRONOUNS,mode);output=next;pronoun_count=count;
 }
 (output,json!({"enabled":true,"classical_context":classical,"kinship_replacements":kinship_count,
  "pronoun_replacements":pronoun_count,"replacements":kinship_count+pronoun_count}))
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn safe_mode_skips_modern_text(){let (v,r)=normalize("你好","Tôi đi học.","safe",true,true,"hanviet");assert_eq!(v,"Tôi đi học.");assert_eq!(r["replacements"],0);}
 #[test]fn strict_mode_normalizes_kinship(){let (v,r)=normalize("第1章 父亲来了","Cha đã đến.","xianxia_strict",true,false,"hanviet");assert_eq!(v,"Phụ thân đã đến.");assert_eq!(r["kinship_replacements"],1);}
 #[test]fn pronouns_are_bounded(){let (v,_)=normalize("本座出手","Ta sẽ ra tay.","xianxia_strict",false,true,"hanviet");assert_eq!(v,"Bản tọa sẽ ra tay.");}
}
