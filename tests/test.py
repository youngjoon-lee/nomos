import subprocess

while True:
	subprocess.run(["cargo", "test", "happy"], check=True) 
