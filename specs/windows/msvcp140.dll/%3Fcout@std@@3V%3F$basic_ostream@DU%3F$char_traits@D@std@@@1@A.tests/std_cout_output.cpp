// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::cout, std::cerr, std::clog and std::cin: output reaches the process's standard output and standard error in order and in sync with the C stdio calls, cin's end of file, and the flush of cout before input (the tie).
#include "poly_wtest.h"
#include <ostream>
#include <iomanip>
#include <sstream>
#include <istream>
#include <streambuf>
#include <locale>
#include <system_error>
#include <new>
#include <stdexcept>
#include <iostream>
void* __cdecl operator new(unsigned long long n) { return malloc(n); }
namespace std { void __cdecl _Facet_Register(_Facet_base*) {} }
static void test(poly::Test& t) {
    std::ios_base::Init init;
    std::cout << "hello " << 42 << ' ' << 1.5 << '\n';
    putchar('A');
    std::cout << 'B';
    putchar('C');
    std::cout << 'D' << '\n';
    std::cout.flush();
    t.check("cout is good after output", std::cout.good(), 0);
    std::cout << "unflushed " << std::hex << 255 << std::dec << ' ' << std::setw(5) << 7 << '\n';
    std::cout << std::flush;
    std::cerr << "to stderr " << -3 << '\n';
    std::clog << "to clog " << true << '\n';
    std::clog << std::flush;
    std::cout << "after cerr\n";
    // tie: reading from cin flushes cout first
    std::cout << "prompt: ";
    int x = 5;
    std::cin >> x;
    t.expect("cin >> x at the end of input leaves x", x, 5);
    t.expect("cin: eofbit and failbit", std::cin.rdstate(), 3);
    std::cin.clear();
    t.expect("cin.peek() at the end of input is EOF", std::cin.peek(), -1);
    t.expect("eofbit", std::cin.rdstate(), 1);
    std::cin.clear();
    t.expect("cin.get() at the end is EOF", std::cin.get(), -1);
    std::cin.clear();
    char buf[8];
    std::cin.getline(buf, 8);
    t.expect("getline at the end fails", std::cin.rdstate(), 3);
    std::cin.clear();
    std::string s;
    std::getline(std::cin, s);
    t.expect("std::getline at the end fails", std::cin.rdstate(), 3);
    std::cout << "end\n";
    std::cout.flush();
}
POLY_TEST_MAIN(test)
