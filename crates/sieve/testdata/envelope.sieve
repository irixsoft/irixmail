require ["envelope", "comparator-i;octet"];
if envelope :domain :is ["from"] "example.com" {
    discard;
}
if envelope :localpart :comparator "i;octet" :matches "to" ["admin*", "postmaster"] {
    keep;
}
