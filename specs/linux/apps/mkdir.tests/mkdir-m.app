{
  "description": "mkdir -m 700 sets the mode; stat shows it",
  "steps": [
    [
      "mkdir",
      "-m",
      "700",
      "d"
    ],
    [
      "stat",
      "-c",
      "%a %F %n",
      "d"
    ]
  ]
}
