require "fileinto";
fileinto "Newsletters";
redirect "elsewhere@example.net";
keep;
stop;
discard;
