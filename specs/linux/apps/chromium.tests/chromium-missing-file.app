{
  "description": "--dump-dom of a file that does not exist: Chromium's error page, exit status 0",
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
      "--dump-dom",
      "file:///work/nope.html"
    ]
  ],
  "files": []
}
