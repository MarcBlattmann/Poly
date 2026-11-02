{
  "description": "headless --dump-dom as root without the sandbox: the DOM after the page's script ran",
  "steps": [
    [
      "chromium",
      "--no-sandbox",
      "--headless=new",
      "--log-level=3",
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
