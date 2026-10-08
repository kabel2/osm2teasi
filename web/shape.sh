#!/bin/sh
# Cap everything the container sends at $RATE.  cake shares that fairly
# between the clients (two downloads get about half each); htb + fq_codel is
# the fallback if the kernel has no cake.  Fails the start rather than serve
# without a limit.
set -e
rate=${RATE:-10mbit}
if ! tc qdisc replace dev eth0 root cake bandwidth "$rate" besteffort 2>/dev/null; then
    tc qdisc replace dev eth0 root handle 1: htb default 1
    tc class replace dev eth0 parent 1: classid 1:1 htb rate "$rate" ceil "$rate"
    tc qdisc replace dev eth0 parent 1:1 fq_codel
fi
echo "shaping eth0: $(tc qdisc show dev eth0 | head -n1)"
