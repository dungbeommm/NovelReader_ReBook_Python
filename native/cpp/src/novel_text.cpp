#include "novel_text.hpp"
#include <unicode/uchar.h>
#include <unicode/unistr.h>
#include <unicode/regex.h>
#include <unicode/locid.h>
#include <algorithm>
#include <memory>
#include <stdexcept>

namespace novel {
namespace {
using icu::UnicodeString;
UnicodeString from(std::string_view text) {
    return UnicodeString::fromUTF8(icu::StringPiece(text.data(), static_cast<int32_t>(text.size())));
}
std::string utf8(const UnicodeString& value) {
    std::string out;
    value.toUTF8String(out);
    return out;
}
UnicodeString strip(const UnicodeString& s) {
    int32_t start = 0;
    int32_t end = s.length();
    while (start < end && u_isUWhiteSpace(s.char32At(start))) start = s.moveIndex32(start, 1);
    while (end > start && u_isUWhiteSpace(s.char32At(s.moveIndex32(end, -1)))) end = s.moveIndex32(end, -1);
    return s.tempSubStringBetween(start, end);
}
std::vector<UnicodeString> lines_of(const UnicodeString& s) {
    std::vector<UnicodeString> lines;
    int32_t start = 0;
    for (int32_t i = 0; i <= s.length(); ++i) {
        if (i == s.length() || s.charAt(i) == u'\n') {
            const auto line = strip(s.tempSubStringBetween(start, i));
            if (!line.isEmpty()) lines.push_back(line);
            start = i + 1;
        }
    }
    return lines;
}
std::string joined(const std::vector<std::string>& lines, std::size_t start, std::size_t end) {
    std::string result;
    for (auto i = start; i < end; ++i) {
        if (!result.empty()) result += "\n";
        result += lines[i];
    }
    return result;
}
bool heading(const UnicodeString& s, const icu::RegexPattern& re) {
    UErrorCode status = U_ZERO_ERROR;
    auto matcher = std::unique_ptr<icu::RegexMatcher>(re.matcher(s, status));
    if (U_FAILURE(status)) throw std::runtime_error("ICU heading matcher failed");
    return matcher->matches(status);
}
bool looks_like_toc(const std::string& title, const std::string& body, const icu::RegexPattern& re) {
    const auto t = from(title);
    if (t.indexOf(from("目录")) >= 0 || t.indexOf(from("目錄")) >= 0 ||
        t.indexOf(from("mục lục")) >= 0 || t == from("toc")) return true;
    const auto lines = lines_of(from(body));
    if (lines.size() < 3) return false;
    std::size_t matches = 0;
    for (const auto& line : lines) {
        if (heading(line, re) || line.startsWith(from("Chương "))) ++matches;
    }
    return matches * 10 >= lines.size() * 4;
}
}  // namespace

std::string normalize(std::string_view input) {
    const auto in = from(input);
    UnicodeString cleaned;
    for (int32_t i = 0; i < in.length();) {
        const UChar32 c = in.char32At(i);
        i = in.moveIndex32(i, 1);
        if (c == 0xfeff) continue;
        if (c == '\r') {
            cleaned.append(u'\n');
            if (i < in.length() && in.charAt(i) == u'\n') ++i;
        } else if (c == 0x201c || c == 0x201d) cleaned.append(u'"');
        else if (c == 0x3000) cleaned.append(u' ');
        else cleaned.append(c);
    }
    const auto lines = lines_of(cleaned);
    UnicodeString out;
    for (const auto& line : lines) {
        if (!out.isEmpty()) out.append(u'\n');
        out.append(line);
    }
    return utf8(out);
}
std::vector<std::string> paragraphs(std::string_view input) {
    const auto lines = lines_of(from(normalize(input)));
    std::vector<std::string> result;
    result.reserve(lines.size());
    for (const auto& line : lines) result.push_back(utf8(line));
    return result;
}
std::vector<Chapter> split_chapters(std::string_view input) {
    const auto lines = paragraphs(input);
    if (lines.empty()) throw std::invalid_argument("Tệp không có nội dung đọc được.");
    UErrorCode status = U_ZERO_ERROR;
    auto pattern = std::unique_ptr<icu::RegexPattern>(icu::RegexPattern::compile(
        from(R"(^(?:第[零〇一二三四五六七八九十百千万两兩億亿0-9]+[章节節回卷].*|(?i:Chapter)\s+[0-9]+.*)$)"), 0, status));
    if (U_FAILURE(status)) throw std::runtime_error("ICU chapter regex failed");
    std::vector<std::size_t> positions;
    for (std::size_t i = 0; i < lines.size(); ++i)
        if (heading(from(lines[i]), *pattern)) positions.push_back(i);
    std::vector<Chapter> result;
    if (!positions.empty()) {
        if (positions.front() > 0) {
            auto pre = joined(lines, 0, positions.front());
            if (!looks_like_toc("Lời mở đầu", pre, *pattern)) result.push_back({"Lời mở đầu", pre});
        }
        for (std::size_t n = 0; n < positions.size(); ++n) {
            const auto start = positions[n], end = n + 1 < positions.size() ? positions[n + 1] : lines.size();
            auto body = joined(lines, start + 1, end);
            if (!looks_like_toc(lines[start], body, *pattern)) result.push_back({lines[start], body});
        }
    } else {
        const auto joined_text = from(joined(lines, 0, lines.size()));
        for (int32_t begin = 0, part = 1; begin < joined_text.length(); ++part) {
            int32_t end = joined_text.moveIndex32(begin, 3000);
            if (end < joined_text.length()) {
                for (int32_t i = end; i > joined_text.moveIndex32(begin, 1500); i = joined_text.moveIndex32(i, -1)) {
                    if (joined_text.charAt(i) == u'\n') { end = i; break; }
                }
            }
            if (end <= begin) end = joined_text.moveIndex32(begin, 3000);
            const auto body = strip(joined_text.tempSubStringBetween(begin, end));
            if (!body.isEmpty()) result.push_back({"Phần " + std::to_string(part), utf8(body)});
            begin = end;
            while (begin < joined_text.length() && joined_text.charAt(begin) == u'\n') ++begin;
        }
    }
    if (result.empty()) throw std::invalid_argument("Không tìm thấy chương hợp lệ.");
    return result;
}
std::vector<std::string> segment(std::string_view input, std::size_t max_codepoints) {
    if (!max_codepoints) throw std::invalid_argument("max_codepoints must be positive");
    const auto value = from(input);
    std::vector<std::string> result;
    for (int32_t begin = 0; begin < value.length();) {
        const int32_t max_end = value.moveIndex32(begin, static_cast<int32_t>(std::min<std::size_t>(max_codepoints, 1000000)));
        int32_t end = max_end;
        if (max_end < value.length()) {
            int32_t punct = -1;
            for (int32_t i = begin; i < max_end; i = value.moveIndex32(i, 1)) {
                const UChar32 c = value.char32At(i);
                if (c == 0x3002 || c == 0xff01 || c == 0xff1f || c == 0xff1b || c == '!' || c == '?' || c == ';')
                    punct = value.moveIndex32(i, 1);
            }
            if (punct > begin) end = punct;
        }
        if (end <= begin) throw std::runtime_error("non-progressing segment");
        result.push_back(utf8(value.tempSubStringBetween(begin, end)));
        begin = end;
    }
    return result;
}
std::string postprocess(std::string_view input) {
    auto s = from(input);
    for (int32_t i = 0; i < s.length(); i = s.moveIndex32(i, 1)) {
        const UChar32 cp = s.char32At(i);
        char repl = 0;
        switch (cp) {
            case 0x3002: repl = '.'; break;
            case 0xff0c: repl = ','; break;
            case 0xff01: repl = '!'; break;
            case 0xff1f: repl = '?'; break;
            case 0xff1b: repl = ';'; break;
            case 0xff1a: repl = ':'; break;
            default: break;
        }
        if (repl) s.replace(i, U16_LENGTH(cp), UnicodeString(repl));
    }
    UErrorCode status = U_ZERO_ERROR;
    auto re = std::unique_ptr<icu::RegexPattern>(icu::RegexPattern::compile(from(R"(\s+([,.!?;:]))"), 0, status));
    if (U_FAILURE(status)) throw std::runtime_error("ICU punctuation regex failed");
    auto m = std::unique_ptr<icu::RegexMatcher>(re->matcher(s, status));
    s = m->replaceAll(from("$1"), status);
    if (U_FAILURE(status)) throw std::runtime_error("ICU punctuation replacement failed");
    if (!s.isEmpty()) {
        auto first = s.tempSubString(0, s.moveIndex32(0, 1));
        first.toUpper(icu::Locale("vi"));
        s.replace(0, s.moveIndex32(0, 1), first);
    }
    return utf8(s);
}
}  // namespace novel
