{
  "description": "sed s///g replaces every match",
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
