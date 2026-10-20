#!/bin/sh
# Rebuilds the PE fixtures from the sources next to this script with
# clang + lld-link (LLVM 18). Neither needs a Windows SDK: the programs are
# freestanding and the import libraries come from .def files.
# The outputs are committed, so the tests never run this.
set -eu
cd "$(dirname "$0")"
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
cflags="--target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-stack-protector -fno-builtin -mno-incremental-linker-compatible -c"
clang $cflags -o "$out/dll.obj" fixture_dll.c
clang $cflags -o "$out/exe.obj" fixture_exe.c
llvm-rc /fo "$out/exe.res" fixture_exe.rc
lld-link /nologo /dll /nodefaultlib /noentry /def:fixture_dll.def /out:fixture.dll \
    /implib:"$out/fixture.lib" /dynamicbase /highentropyva /brepro "$out/dll.obj"
llvm-dlltool -m i386:x86-64 -d delayed.def -l "$out/delayed.lib"
lld-link /nologo /nodefaultlib /subsystem:console /entry:mainCRTStartup /out:fixture.exe \
    /dynamicbase /highentropyva /nxcompat /brepro /debug /pdb:"$out/fixture.pdb" /pdbaltpath:fixture.pdb \
    /delayload:delayed.dll "$out/exe.obj" "$out/exe.res" "$out/fixture.lib" "$out/delayed.lib"
# A COFF object with a section name longer than 8 bytes (kept in the
# string table).
printf '__attribute__((section(".poly_long_name"))) int poly_value = 1;\n' > "$out/obj.c"
clang $cflags -o fixture.obj "$out/obj.c"
