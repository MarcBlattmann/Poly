// The C++ standard library on Poly (roadmap Phase 4, ADR 0049): a
// dynamically linked C++ program built at test time with the host's C++
// compiler, so the dynamic linker loads libstdc++, libm, libgcc_s and libc.
// Containers, strings, iostreams, a shared pointer, a static object's
// constructor and destructor, and an exception thrown and caught (the
// unwinder finds the frames through the loaded objects' unwind tables).
//
// Prints "dyn-cxx: ok" and exits 0; on a failed check it prints what failed
// and exits 1. Written for Poly from the C++ standard's library clauses.

#include <algorithm>
#include <iostream>
#include <map>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

int constructed = 0;

struct Global {
    Global() { constructed++; }
};

Global global;

[[noreturn]] void thrower(int depth) {
    if (depth == 0) {
        throw std::runtime_error("thrown through frames");
    }
    std::string keep = "frame " + std::to_string(depth);
    thrower(depth - 1);
}

}  // namespace

int main() {
    bool ok = true;
    auto check = [&ok](bool good, const char *what) {
        if (!good) {
            std::cout << "dyn-cxx: FAIL " << what << std::endl;
            ok = false;
        }
    };
    check(constructed == 1, "a static object's constructor");

    std::vector<std::string> words{"poly", "runs", "glibc", "unchanged"};
    std::sort(words.begin(), words.end());
    check(words.front() == "glibc" && words.back() == "unchanged", "sorting strings");

    std::map<std::string, int> lengths;
    for (const auto &w : words) {
        lengths[w] = static_cast<int>(w.size());
    }
    check(lengths.at("unchanged") == 9 && lengths.size() == 4, "a map");

    std::ostringstream out;
    out << 6 * 7 << ' ' << 1.5;
    check(out.str() == "42 1.5", "a string stream");

    auto shared = std::make_shared<std::vector<int>>(1000, 7);
    std::weak_ptr<std::vector<int>> weak = shared;
    check(weak.lock()->at(999) == 7, "shared and weak pointers");
    shared.reset();
    check(weak.expired(), "the last shared pointer frees");

    bool caught = false;
    try {
        thrower(5);
    } catch (const std::exception &e) {
        caught = std::string(e.what()) == "thrown through frames";
    }
    check(caught, "an exception thrown and caught");

    bool out_of_range = false;
    try {
        (void)words.at(10);
    } catch (const std::out_of_range &) {
        out_of_range = true;
    }
    check(out_of_range, "std::out_of_range from the library");

    if (!ok) {
        return 1;
    }
    std::cout << "dyn-cxx: ok" << std::endl;
    return 0;
}
