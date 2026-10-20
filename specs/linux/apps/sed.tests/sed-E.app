{
  "description": "sed -E uses extended regular expressions with groups",
  "steps": [
    [
      "sed",
      "-E",
      "s/(.)(.*)/\\2\\1/",
      "fruit.txt"
    ]
  ],
  "files": [
    {
      "path": "fruit.txt",
      "text": "pear\napple\nfig\nbanana\napple\ncherry\n"
    }
  ]
}
