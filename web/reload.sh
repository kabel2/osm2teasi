#!/bin/sh
# certbot renews the certificate in place; nginx only reads it on (re)load
(while sleep 86400; do nginx -s reload; done) &
