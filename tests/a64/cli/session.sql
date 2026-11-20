-- An SQLite session for Poly's arm64 path (ADR 0257 addendum), run by the
-- SQLite shell on an in-memory database. Its output is compared line by
-- line with session.out.
.headers off
.mode list
SELECT 'sqlite ' || substr(sqlite_version(), 1, 4);
CREATE TABLE city(id INTEGER PRIMARY KEY, name TEXT NOT NULL, country TEXT, people INTEGER);
INSERT INTO city(name, country, people) VALUES
  ('Zurich', 'CH', 421878), ('Geneva', 'CH', 203856), ('Basel', 'CH', 173863),
  ('Lyon', 'FR', 522250), ('Paris', 'FR', 2102650), ('Milan', 'IT', 1371498);
CREATE INDEX city_country ON city(country);
SELECT country, count(*), sum(people) FROM city GROUP BY country ORDER BY country;
SELECT name FROM city WHERE people > 400000 ORDER BY people DESC;
SELECT name, rank() OVER (PARTITION BY country ORDER BY people DESC) FROM city ORDER BY country, 2;
WITH RECURSIVE fib(n, a, b) AS (SELECT 1, 0, 1 UNION ALL SELECT n + 1, b, a + b FROM fib WHERE n < 20)
  SELECT group_concat(a, ',') FROM fib;
SELECT printf('%.3f|%5d|%-4s|%x', 3.14159, 42, 'ab', 255);
SELECT json_object('name', name, 'people', people) FROM city WHERE id = 1;
SELECT json_extract('{"a":[1,2,{"b":"x"}]}', '$.a[2].b');
SELECT date('2026-11-20', '+1 month', 'start of month'), strftime('%j', '2000-12-31');
SELECT typeof(1), typeof(1.5), typeof('x'), typeof(x'00'), typeof(NULL);
BEGIN;
UPDATE city SET people = people + 1 WHERE country = 'CH';
ROLLBACK;
SELECT sum(people) FROM city WHERE country = 'CH';
CREATE TABLE t(x);
INSERT INTO t SELECT value FROM generate_series(1, 10000);
SELECT count(*), sum(x), avg(x), min(x), max(x) FROM t;
SELECT abs(-5), round(3.14159, 2), upper('mixed Case'), length('héllo'), hex('Poly');
SELECT 'done';
