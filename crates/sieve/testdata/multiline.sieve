require ["fileinto", "encoded-character"];
# hash comment
/* bracket
   comment */
if header :is "subject" text: # note after text:
line one
..stuffed
.
{
    fileinto "${hex:40}archive";
}
