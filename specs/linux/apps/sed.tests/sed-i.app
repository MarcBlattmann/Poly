{
  "description": "sed -i edits a file in place; cat shows it",
  "steps": [
    [
      "sed",
      "-i",
      "s/fig/FIG/",
      "fruit.txt"
    ],
    [
      "cat",
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
