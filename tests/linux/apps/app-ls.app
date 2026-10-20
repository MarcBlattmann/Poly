{
  "description": "ls(1) -la lists a fixed directory tree in long format",
  "steps": [
    [
      "ls",
      "-la",
      "tree"
    ]
  ],
  "files": [
    {
      "path": "tree/notes.txt",
      "text": "some notes\n"
    },
    {
      "path": "tree/empty",
      "text": ""
    },
    {
      "path": "tree/run.sh",
      "text": "#!/bin/sh\necho hi\n",
      "mode": "755"
    },
    {
      "path": "tree/sub",
      "dir": true
    },
    {
      "path": "tree/sub/deep.txt",
      "text": "deep\n"
    },
    {
      "path": "tree/link",
      "symlink": "notes.txt"
    }
  ]
}
