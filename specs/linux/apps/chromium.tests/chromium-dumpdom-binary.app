{
  "description": "the browser binary itself (not the start script), as poly with the sandbox, --dump-dom of a page that sorts, serialises JSON and does floating point",
  "steps": [
    [
      "setpriv",
      "--reuid=1000",
      "--regid=1000",
      "--clear-groups",
      "env",
      "HOME=/tmp/poly-home",
      "/usr/lib/chromium/chromium",
      "--headless=new",
      "--log-level=3",
      "--dump-dom",
      "file:///work/sort.html"
    ]
  ],
  "files": [
    {
      "path": "sort.html",
      "text": "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>sorting</title></head><body>\n<ul id=\"l\"></ul><pre id=\"j\"></pre>\n<script>\nvar words = ['pear', 'apple', 'fig', 'banana', 'cherry'];\nwords.sort().forEach(function (w) { var li = document.createElement('li'); li.textContent = w; document.getElementById('l').appendChild(li); });\nvar o = {n: 42, list: [1, 2, 3].map(function (x) { return x * x; }), s: 'caf\\u00e9'};\ndocument.getElementById('j').textContent = JSON.stringify(o) + ' ' + (0.1 + 0.2).toFixed(17) + ' ' + Math.sqrt(2);\ndocument.title = 'sorted ' + words.length;\n</script></body></html>\n"
    }
  ]
}
