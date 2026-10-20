{
  "description": "sort -t: -k3 sorts by the third colon-separated field",
  "steps": [
    [
      "sort",
      "-t:",
      "-k3",
      "p.txt"
    ]
  ],
  "files": [
    {
      "path": "p.txt",
      "text": "x:1:zeta\ny:2:alpha\nz:3:mid\n"
    }
  ]
}
