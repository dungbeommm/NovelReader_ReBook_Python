#include "novel_text.hpp"
#include <cassert>
#include <iostream>
#include <stdexcept>

int main() {
    assert(novel::normalize("\xef\xbb\xbf 你好。\r\n \r\n 世界。") == "你好。\n世界。");
    const auto parts = novel::split_chapters("第1章 开始\n你好。\n第2章 继续\n再见。");
    assert(parts.size() == 2);
    assert(parts[0].title == "第1章 开始");
    assert(parts[1].body == "再见。");
    const std::string input = "青山绿水。明天再会！这里有文字。";
    std::string output;
    for (const auto& piece : novel::segment(input, 5)) output += piece;
    assert(output == input);
    assert(novel::postprocess("你好， 世界。") == "你好, 世界.");
    bool thrown = false;
    try { (void)novel::split_chapters(" \n "); } catch (const std::invalid_argument&) { thrown = true; }
    assert(thrown);
    std::cout << "native text tests: OK\n";
}
