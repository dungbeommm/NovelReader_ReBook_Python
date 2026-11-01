#pragma once
#include <cstddef>
#include <string>
#include <string_view>
#include <vector>

namespace novel {
struct Chapter {
    std::string title;
    std::string body;
};
std::string normalize(std::string_view text);
std::vector<std::string> paragraphs(std::string_view text);
std::vector<Chapter> split_chapters(std::string_view text);
std::vector<std::string> segment(std::string_view text, std::size_t max_codepoints = 200);
std::string postprocess(std::string_view text);
}  // namespace novel
