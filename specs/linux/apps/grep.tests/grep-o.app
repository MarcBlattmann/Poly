{
  "description": "grep -o prints only the matching parts",
  "steps": [
    [
      "grep",
      "-o",
      "p[a-z]",
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
