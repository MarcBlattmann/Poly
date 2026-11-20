-- A Lua session for Poly's arm64 path (ADR 0257 addendum): the language,
-- the standard libraries, the C library under them. Each check prints
-- "ok <what>" or "not ok <what>"; the last line counts the failures.
local failed = 0
local function check(what, ok)
  print((ok and "ok " or "not ok ") .. what)
  if not ok then failed = failed + 1 end
end

print("lua: " .. _VERSION .. " on " .. (jit and "LuaJIT" or "the reference interpreter"))

-- Strings, patterns, formatting (snprintf with %.14g, strtod).
check("string.format", string.format("%5.2f|%d|%s|%q", 3.14159, 42, "x", "a\nb") == " 3.14|42|x|\"a\\\nb\"")
check("number to string", tostring(1/3) == "0.33333333333333" and tostring(1e100) == "1e+100" and tostring(2^53) == "9.007199254741e+15")
check("string to number", tonumber("0x1p4") == 16 and tonumber("  12.5e1 ") == 125 and tonumber("1e") == nil)
check("gsub and patterns", ("hello world"):gsub("o", "0") == "hell0 w0rld" and ("k=v, a=b"):match("(%w+)=(%w+)") == "k")
check("string.rep and upper", ("ab"):rep(3, "-") == "ab-ab-ab" and ("MiXeD"):upper() == "MIXED")

-- Tables, sorting, closures, metatables.
local t = {}
for i = 1, 1000 do t[i] = (i * 7919) % 1000 end
table.sort(t)
local sorted = true
for i = 2, #t do if t[i - 1] > t[i] then sorted = false end end
check("table.sort of 1000 numbers", sorted and #t == 1000)
local function counter() local n = 0; return function() n = n + 1; return n end end
local c = counter(); c(); c()
check("closures", c() == 3)
local V = setmetatable({}, { __index = function(_, k) return k * 2 end, __call = function(_, x) return x + 1 end })
check("metatables", V[21] == 42 and V(1) == 2)

-- Coroutines, errors.
local co = coroutine.wrap(function(a) local b = coroutine.yield(a + 1); return b * 2 end)
check("coroutines", co(1) == 2 and co(5) == 10)
local ok, err = pcall(function() error({ code = 7 }) end)
check("pcall and error", not ok and err.code == 7)
local ok2, msg = pcall(function() return nil + 1 end)
check("runtime errors", not ok2 and msg:find("arithmetic") ~= nil)

-- Math (libsystem_m).
check("math", math.floor(-2.5) == -3 and math.ceil(2.1) == 3 and math.abs(math.sin(math.pi)) < 1e-15
  and math.sqrt(2)^2 - 2 < 1e-15 and math.fmod(7, 3) == 1 and math.max(1, 5, 3) == 5 and math.type(1 // 1) == "integer")
check("integer arithmetic", 7 // 2 == 3 and -7 // 2 == -4 and 7 % -3 == -2 and math.maxinteger + 1 == math.mininteger)

-- The operating system and files (fopen, fread, fseek, remove, time).
local name = os.tmpname and "/tmp/poly-lua-test.txt" or "/tmp/poly-lua-test.txt"
local f = assert(io.open(name, "w"))
f:write("line one\n", 2, "\n", 3.5, "\nlast")
f:close()
local lines = {}
for l in io.lines(name) do lines[#lines + 1] = l end
check("io.open, write, lines", #lines == 4 and lines[1] == "line one" and lines[2] == "2" and lines[4] == "last")
f = assert(io.open(name, "r"))
f:seek("set", 5)
local word = f:read(3)
local rest = f:read("a")
check("seek and read", word == "one" and rest:sub(1, 1) == "\n" and f:seek("cur") == 19)
f:close()
check("os.remove", os.remove(name) == true and io.open(name, "r") == nil)
local now = os.time()
check("os.time", now > 1700000000)
check("os.date", os.date("!%Y-%m-%d", 0) == "1970-01-01" and os.date("!%c", 86400 * 365) == "Fri Jan  1 00:00:00 1971")
check("os.clock", type(os.clock()) == "number" and os.clock() >= 0)
check("os.getenv", os.getenv("POLY_LUA") == "yes" and os.getenv("NOPE_NOT_SET") == nil)

print("lua: done, " .. failed .. " failed")
os.exit(failed == 0 and 0 or 1)
