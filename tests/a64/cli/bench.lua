-- A small benchmark for the arm64 translator (ADR 0257 addendum): the
-- kinds of work Lua programs do. Prints one checksum per part, so a run
-- under the JIT and one under the interpreter can be compared exactly.
local function fib(n) if n < 2 then return n end return fib(n - 1) + fib(n - 2) end

local function tables(n)
  local t = {}
  for i = 1, n do t[i] = i * 2 end
  local s = 0
  for _, v in ipairs(t) do s = s + v end
  local h = {}
  for i = 1, n do h["k" .. (i % 500)] = (h["k" .. (i % 500)] or 0) + i end
  for _, v in pairs(h) do s = s + v end
  return s
end

local function strings(n)
  local parts = {}
  for i = 1, n do parts[#parts + 1] = string.format("%d:%x", i, i * 7) end
  local s = table.concat(parts, ",")
  local count = 0
  for w in s:gmatch("%d+:") do count = count + #w end
  return count + #s:upper()
end

local function closures(n)
  local acc = 0
  local function make(k) return function(x) return x * k + 1 end end
  for i = 1, n do acc = (acc + make(i % 7)(i)) % 1000003 end
  return acc
end

local function floats(n)
  local x = 0.0
  for i = 1, n do x = x + math.sin(i) * math.sqrt(i) / (1 + i % 3) end
  return math.floor(x * 1000)
end

local function sort(n)
  local t, seed = {}, 42
  for i = 1, n do seed = (seed * 1103515245 + 12345) % 2147483648; t[i] = seed end
  table.sort(t)
  return t[1] + t[n // 2] + t[n]
end

print("bench: fib", fib(24))
print("bench: tables", tables(30000))
print("bench: strings", strings(4000))
print("bench: closures", closures(40000))
print("bench: floats", floats(20000))
print("bench: sort", sort(20000))
print("bench: done")
