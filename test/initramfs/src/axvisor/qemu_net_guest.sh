#!/bin/sh

set -eu

ip link set eth0 up
ip addr add 172.16.0.2/24 dev eth0
wget -T 20 -q -O /tmp/qemu-network-probe http://172.16.0.1:8080/probe
grep -q '^qemu-network-pass$' /tmp/qemu-network-probe
echo 'qemu kvm guest test pass!'
