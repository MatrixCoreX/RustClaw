#!/usr/bin/perl

# Perl example script
use strict;
use warnings;

print "Hello from Perl!\n";

# Simple loop example
for my $i (1..5) {
    print "Iteration $i\n";
}

# Hash example
my %capitals = (
    'France'  => 'Paris',
    'Japan'   => 'Tokyo',
    'Brazil'  => 'Brasília'
);

print "Capital of Japan: $capitals{'Japan'}\n";
