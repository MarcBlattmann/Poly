{
  "description": "cp -r copies a directory tree; ls -R lists it",
  "steps": [
    [
      "cp",
      "-r",
      "src",
      "dst"
    ],
    [
      "ls",
      "-R",
      "dst"
    ]
  ],
  "files": [
    {
      "path": "src/a",
      "text": "a\n"
    },
    {
      "path": "src/sub/b",
      "text": "b\n"
    }
  ]
}
