{
  "description": "wc reports a missing file and exits 1",
  "steps": [
    [
      "wc",
      "fruit.txt",
      "nope"
    ]
  ],
  "files": [
    {
      "path": "fruit.txt",
      "text": "pear\napple\nfig\nbanana\napple\ncherry\n"
    }
  ]
}
