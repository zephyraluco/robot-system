#!/bin/bash

set -e
script=$(readlink -f "$0")
route=$(dirname "$script")

cd ${route}/..

cargo build --release