require "fileinto";
if address :domain :matches "from" ["*.example.com", "example.?rg"] {
    fileinto "Partners";
}
if header :matches "list-id" "*<deals.*>*" {
    fileinto "Deals";
}
