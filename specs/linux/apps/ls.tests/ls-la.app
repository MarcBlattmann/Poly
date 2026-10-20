{
  "description": "ls -la lists a tree in long format with dot files",
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
      "path": "tree/big.txt",
      "text": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n"
    },
    {
      "path": "tree/.hidden",
      "text": "h\n"
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
    },
    {
      "path": "tree/dangling",
      "symlink": "missing-target"
    }
  ]
}
