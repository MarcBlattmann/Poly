{
  "description": "sed(1) replaces every match of a regular expression on every line",
  "steps": [
    [
      "sed",
      "s/a/A/g",
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
