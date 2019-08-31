#!/bin/sh

START=$(date '+%H:%M:%S')
DELAY=${1:-5}
echo -n "$0 (PID $$): "
sleep $DELAY
END=
echo "start $START, end $(date '+%H:%M:%S')"
