{
  "description": "sed 1a appends a line after the first",
  "steps": [
    [
      "sed",
      "1a\\added",
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
