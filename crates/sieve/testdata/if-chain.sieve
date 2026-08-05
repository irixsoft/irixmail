require "fileinto";
if header :is "subject" "invoice" {
    fileinto "Invoices";
} elsif header :contains ["subject", "from"] "newsletter" {
    fileinto "Newsletters";
} else {
    keep;
}
