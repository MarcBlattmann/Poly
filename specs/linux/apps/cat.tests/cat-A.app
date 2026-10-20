{
  "description": "cat -A shows tabs, line ends and non-printing bytes",
  "steps": [
    [
      "cat",
      "-A",
      "odd.txt"
    ]
  ],
  "files": [
    {
      "path": "odd.txt",
      "text": "a\tb\u0001c\nend\n"
    }
  ]
}
