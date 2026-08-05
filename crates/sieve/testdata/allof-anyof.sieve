if allof (exists "from", anyof (header :contains "subject" "a", header :contains "subject" "b")) {
    discard;
}
if not allof (exists "list-id", not exists "precedence") {
    keep;
}
