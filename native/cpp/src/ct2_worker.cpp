#include <ctranslate2/translator.h>
#include <ctranslate2/translation.h>
#include <sentencepiece_processor.h>
#include <nlohmann/json.hpp>
#include <algorithm>
#include <iostream>
#include <string>
#include <vector>
using json=nlohmann::json;

static std::string restore_newline_marker(std::string text,const std::string& marker) {
    if(marker.empty()) return text;
    std::size_t pos=0;
    while((pos=text.find(marker,pos))!=std::string::npos){
        std::size_t end=pos+marker.size();
        while(end<text.size()&&(text[end]==' '||text[end]=='\t'||text[end]=='\r'))++end;
        text.replace(pos,end-pos,"\n");++pos;
    }
    return text;
}
static std::string invert_opener_spacing(std::string text) {
    const std::string openers="“‘«「『【《([{（［｛";
    for(std::size_t i=0;i+4<text.size();++i){
        if(text.compare(i,3,"“")==0||text.compare(i,3,"‘")==0||text.compare(i,3,"「")==0||
           text.compare(i,3,"『")==0||text.compare(i,3,"【")==0||text.compare(i,3,"《")==0){
            const auto space=i+3;
            if(space+1<text.size()&&text[space]==' '){
                const unsigned char c=static_cast<unsigned char>(text[space+1]);
                if(c>=0xC0||((c>='A'&&c<='Z')||(c>='a'&&c<='z'))) text.erase(space,1);
            }
        } else if(openers.find(text[i])!=std::string::npos&&text[i+1]==' '){
            const unsigned char c=static_cast<unsigned char>(text[i+2]);
            if(c>=0xC0||((c>='A'&&c<='Z')||(c>='a'&&c<='z'))) text.erase(i+1,1);
        }
    }
    return text;
}
static std::string translate_one(ctranslate2::Translator& translator,
    sentencepiece::SentencePieceProcessor& source,sentencepiece::SentencePieceProcessor& target,
    const json& request,const std::string& newline_marker,bool opener_spacing) {
    const auto text=request.value("text",std::string{});
    std::string source_text=text;
    if(!newline_marker.empty()){
        std::string normalized;normalized.reserve(source_text.size());
        for(std::size_t i=0;i<source_text.size();++i){
            if(source_text[i]=='\r'){if(i+1<source_text.size()&&source_text[i+1]=='\n')++i;normalized+=newline_marker;}
            else if(source_text[i]=='\n') normalized+=newline_marker;
            else normalized+=source_text[i];
        }
        source_text=std::move(normalized);
        const auto marker_pos=source_text.find(newline_marker);
        if(marker_pos!=std::string::npos&&marker_pos+newline_marker.size()<source_text.size()&&source_text.compare(marker_pos+newline_marker.size(),newline_marker.size(),newline_marker)!=0)
            source_text.insert(marker_pos+newline_marker.size()," ");
    }
    std::vector<std::string> tokens;
    auto status=source.Encode(source_text,&tokens);
    if(!status.ok()) throw std::runtime_error(status.ToString());
    const auto max_input=std::max<std::size_t>(1,request.value("max_input_tokens",160u));
    if(tokens.size()+1>max_input) throw std::runtime_error("Input chunk vượt giới hạn token; cần chia chunk trước khi dịch.");
    tokens.emplace_back("</s>");
    ctranslate2::TranslationOptions options;
    options.beam_size=std::clamp<std::size_t>(request.value("beam_size",2u),1,4);
    options.max_input_length=max_input;
    options.max_decoding_length=std::max<std::size_t>(1,request.value("max_output_tokens",300u));
    options.repetition_penalty=request.value("repetition_penalty",1.0f);
    options.no_repeat_ngram_size=0;
    const auto results=translator.translate_batch({tokens},options,1,ctranslate2::BatchType::Examples);
    if(results.empty()||results.front().hypotheses.empty()) throw std::runtime_error("Model không trả hypothesis.");
    auto pieces=results.front().hypotheses.front();
    pieces.erase(std::remove_if(pieces.begin(),pieces.end(),[](const std::string& p){
        return p=="<pad>"||p=="<s>"||p=="</s>"||p=="<unk>";
    }),pieces.end());
    std::string decoded;
    auto decode_status=target.Decode(pieces,&decoded);
    if(!decode_status.ok()) throw std::runtime_error(decode_status.ToString());
    if(!newline_marker.empty()) decoded=restore_newline_marker(decoded,newline_marker);
    if(opener_spacing) decoded=invert_opener_spacing(decoded);
    return decoded;
}
int main(int argc,char** argv){
    if(argc<8){std::cerr<<"Usage: novelreader-ct2-worker MODEL SOURCE_SPM TARGET_SPM cpu|cuda COMPUTE DEVICE_INDEX INTRA_THREADS\n";return 2;}
    try{
        const std::string model_path=argv[1],source_path=argv[2],target_path=argv[3],device_name=argv[4],compute_name=argv[5];
        const int device_index=std::stoi(argv[6]);const int threads=std::max(1,std::stoi(argv[7]));
        sentencepiece::SentencePieceProcessor source,target;
        auto s1=source.Load(source_path);if(!s1.ok())throw std::runtime_error("Không nạp source.spm: "+s1.ToString());
        auto s2=target.Load(target_path);if(!s2.ok())throw std::runtime_error("Không nạp target.spm: "+s2.ToString());
        ctranslate2::models::ModelLoader loader(model_path);
        loader.device=ctranslate2::str_to_device(device_name);
        loader.compute_type=ctranslate2::str_to_compute_type(compute_name);
        loader.device_indices={device_index};
        ctranslate2::ReplicaPoolConfig pool;pool.num_threads_per_replica=static_cast<std::size_t>(threads);
        ctranslate2::Translator translator(loader,pool);
        std::cout<<json({{"ready",true},{"device",device_name},{"compute_type",compute_name}}).dump()<<std::endl;
        std::string line;
        while(std::getline(std::cin,line)){
            try{
                const auto request=json::parse(line);
                const auto op=request.value("op",std::string{});
                if(op=="shutdown")break;
                if(op=="count_tokens"){
                    std::string text=request.value("text",std::string{}),marker=request.value("newline_token",std::string{});
                    if(!marker.empty()){
                        std::string normalized;for(std::size_t i=0;i<text.size();++i){
                            if(text[i]=='\r'){if(i+1<text.size()&&text[i+1]=='\n')++i;normalized+=marker;}
                            else if(text[i]=='\n')normalized+=marker;else normalized+=text[i];
                        }text=std::move(normalized);
                    }
                    std::vector<std::string> pieces;auto status=source.Encode(text,&pieces);
                    if(!status.ok())throw std::runtime_error(status.ToString());
                    std::cout<<json({{"ok",true},{"tokens",pieces.size()+1}}).dump()<<std::endl;continue;
                }
                if(op!="translate"){std::cout<<json({{"ok",false},{"error","Unsupported operation"}}).dump()<<std::endl;continue;}
                const std::string marker=request.value("newline_token",std::string{});
                const bool opener=request.value("opener_spacing",false);
                const auto output=translate_one(translator,source,target,request,marker,opener);
                std::cout<<json({{"ok",true},{"text",output},{"device",device_name},{"compute_type",compute_name}}).dump()<<std::endl;
            }catch(const std::exception& e){std::cout<<json({{"ok",false},{"error",e.what()}}).dump()<<std::endl;}
        }
    }catch(const std::exception& e){std::cout<<json({{"ready",false},{"error",e.what()}}).dump()<<std::endl;return 1;}
    return 0;
}
