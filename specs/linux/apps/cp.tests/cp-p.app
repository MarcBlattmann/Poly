{
  "description": "cp -p keeps mode and modification time; ls -l shows them",
  "steps": [
    [
      "cp",
      "-p",
      "run.sh",
      "copy.sh"
    ],
    [
      "ls",
      "-l",
      "copy.sh"
    ]
  ],
  "files": [
    {
      "path": "run.sh",
      "text": "#!/bin/sh\n",
      "mode": "750"
    }
  ]
}
