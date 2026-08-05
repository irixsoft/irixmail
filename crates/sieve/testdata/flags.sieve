require ["fileinto", "imap4flags"];
addflag "\\Seen \\Flagged";
setflag ["one", "two three"];
removeflag "two";
if header :contains "x-priority" "1" {
    addflag "$label:urgent";
    fileinto "Priority";
}
