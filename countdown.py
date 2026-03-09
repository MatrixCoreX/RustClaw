import time
import sys

def countdown(seconds):
    for i in range(seconds, 0, -1):
        print(f"\r{i} seconds remaining...", end="", flush=True)
        time.sleep(1)
    print("\rTime's up! 🎉")

if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python countdown.py <seconds>")
        sys.exit(1)
    
    try:
        secs = int(sys.argv[1])
        if secs <= 0:
            print("Please enter a positive integer.")
            sys.exit(1)
        countdown(secs)
    except ValueError:
        print("Please enter a valid integer.")
        sys.exit(1)