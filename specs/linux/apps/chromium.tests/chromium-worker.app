{
  "description": "a dedicated Web Worker (its own thread) sums 1..1000; --virtual-time-budget lets the page settle before the DOM is dumped",
  "steps": [
    [
      "setpriv",
      "--reuid=1000",
      "--regid=1000",
      "--clear-groups",
      "env",
      "HOME=/tmp/poly-home",
      "chromium",
      "--headless=new",
      "--log-level=3",
      "--virtual-time-budget=5000",
      "--dump-dom",
      "file:///work/worker.html"
    ]
  ],
  "files": [
    {
      "path": "worker.html",
      "text": "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>worker</title></head><body><p id=\"w\">waiting</p>\n<script>\nvar src = 'onmessage = function (e) { var s = 0; for (var i = 1; i <= e.data; i++) s += i; postMessage(s); };';\nvar w = new Worker(URL.createObjectURL(new Blob([src], {type: 'text/javascript'})));\nw.onmessage = function (e) { document.getElementById('w').textContent = 'worker sum ' + e.data; };\nw.postMessage(1000);\n</script></body></html>\n"
    }
  ]
}
