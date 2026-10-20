{
  "description": "sed /re/d deletes matching lines",
  "steps": [
    [
      "sed",
      "/apple/d",
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
