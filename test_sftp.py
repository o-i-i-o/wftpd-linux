#!/usr/bin/env python3
import subprocess
import sys

password = "123456"
commands = ["ls -la", "pwd", "get test.txt /tmp/test_download.txt", "exit"]

try:
    proc = subprocess.Popen(
        ["sftp", "-P", "2222", "-o", "StrictHostKeyChecking=no", 
         "-o", "UserKnownHostsFile=/dev/null", "123@localhost"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    
    input_text = password + "\n" + "\n".join(commands) + "\n"
    stdout, stderr = proc.communicate(input=input_text, timeout=10)
    
    print("=== STDOUT ===")
    print(stdout)
    print("\n=== STDERR ===")
    print(stderr)
    print("\n=== Return Code ===")
    print(proc.returncode)
    
except subprocess.TimeoutExpired:
    proc.kill()
    print("Timeout - process killed")
except Exception as e:
    print(f"Error: {e}")
