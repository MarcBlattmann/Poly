{
  "description": "grep -F matches fixed strings",
  "steps": [
    [
      "grep",
      "-F",
      "a.b",
      "dots.txt"
    ]
  ],
  "files": [
    {
      "path": "dots.txt",
      "text": "a.b\naxb\na.bc\n"
    }
  ]
}
