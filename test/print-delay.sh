#!/bin/bash

START=$(date '+%H:%M:%S')
if [ -f delay.txt ] ; then
    DELAY=$(cat delay.txt)
else
    DELAY=5
fi
echo -n "%0 "
sleep $DELAY
END=$(date '+%H:%M:%S')
echo "$$: start: $START end: $END"
