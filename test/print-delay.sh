#!/bin/bash

START=$(date '+%H:%M:%S')
if [ -f delay.txt ] ; then
    DELAY=$(cat delay.txt)
else
    DELAY=5
fi
echo -n "$0 (PID $$): "
sleep $DELAY
END=
echo "start $START, end $(date '+%H:%M:%S')"
