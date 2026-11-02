{
  "description": "--disable-namespace-sandbox makes Chromium use its setuid sandbox helper (chrome-sandbox, mode 4755) even where user namespaces are allowed",
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
      "--disable-namespace-sandbox",
      "--dump-dom",
      "file:///work/hello.html"
    ]
  ],
  "files": [
    {
      "path": "hello.html",
      "text": "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Poly Chromium hello</title></head>\n<body><h1 id=\"h\">Hello from Poly</h1>\n<p id=\"p\"></p>\n<script>document.getElementById('p').textContent = 'script ran: ' + (6 * 7);</script>\n</body></html>\n"
    }
  ]
}
