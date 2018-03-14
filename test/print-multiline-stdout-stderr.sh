#!/bin/sh

for i in $(seq 5) ; do
    if [ $(expr $i % 2) == 0 ] ; then
	echo this is standard error
    else
	>&2 echo this is standard out
    fi
    sleep 1
done
