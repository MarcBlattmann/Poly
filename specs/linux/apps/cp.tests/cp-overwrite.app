{
  "description": "cp replaces the contents of an existing target",
  "steps": [
    [
      "cp",
      "a",
      "b"
    ],
    [
      "cat",
      "b"
    ]
  ],
  "files": [
    {
      "path": "a",
      "text": "new\n"
    },
    {
      "path": "b",
      "text": "old old old\n"
    }
  ]
}
