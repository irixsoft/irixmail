if size :over 1M {
    discard;
}
if size :under 10k {
    keep;
}
if allof (true, exists ["from", "date"], not false) {
    stop;
}
