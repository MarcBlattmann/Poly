{
  "description": "grep exits 1 when nothing matches",
  "steps": [
    [
      "grep",
      "kiwi",
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
