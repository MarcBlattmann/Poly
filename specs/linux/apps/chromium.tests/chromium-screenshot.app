{
  "description": "headless --screenshot of a page of four coloured quadrants (400x300): the PNG's size header and its SHA-256 (the same pixels give the same bytes)",
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
      "--screenshot=/tmp/poly-home/shot.png",
      "--window-size=400,300",
      "--hide-scrollbars",
      "file:///work/colours.html"
    ],
    [
      "od",
      "-An",
      "-tx1",
      "-j16",
      "-N8",
      "/tmp/poly-home/shot.png"
    ],
    [
      "sha256sum",
      "/tmp/poly-home/shot.png"
    ]
  ],
  "files": [
    {
      "path": "colours.html",
      "text": "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Poly Chromium colours</title>\n<style>html,body{margin:0;padding:0;overflow:hidden}\ndiv{position:absolute;width:50vw;height:50vh}</style></head>\n<body><div style=\"left:0;top:0;background:#ff0000\"></div><div style=\"left:50vw;top:0;background:#00ff00\"></div><div style=\"left:0;top:50vh;background:#0000ff\"></div><div style=\"left:50vw;top:50vh;background:#ffff00\"></div></body></html>\n"
    }
  ]
}
