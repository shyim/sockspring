#!/usr/bin/env python3
"""Create a local Play upload key; never print passwords or overwrite an existing key."""
import os
from pathlib import Path
import secrets
import subprocess

root = Path(__file__).resolve().parents[1]
folder = root / '.signing'
props = root / 'upload-signing.properties'
key = folder / 'sockspring-upload.p12'
if props.exists() or key.exists():
    raise SystemExit('Signing files already exist; refusing to overwrite.')
os.umask(0o077)
folder.mkdir(mode=0o700, exist_ok=True)
password = secrets.token_hex(32)
env = dict(os.environ, SOCKSPRING_KEY_PASSWORD=password)
keytool = str(Path(os.environ['JAVA_HOME']) / 'bin/keytool') if 'JAVA_HOME' in os.environ else 'keytool'
subprocess.run([keytool, '-genkeypair', '-keystore', str(key), '-storetype', 'PKCS12',
    '-storepass:env', 'SOCKSPRING_KEY_PASSWORD', '-keypass:env', 'SOCKSPRING_KEY_PASSWORD',
    '-alias', 'upload', '-keyalg', 'RSA', '-keysize', '4096', '-validity', '10000',
    '-dname', 'CN=Sockspring Upload, OU=Android, O=Shyim', '-noprompt'], env=env, check=True)
props.write_text(f'storeFile=.signing/sockspring-upload.p12\nstorePassword={password}\nkeyAlias=upload\nkeyPassword={password}\n')
subprocess.run([keytool, '-exportcert', '-rfc', '-keystore', str(key), '-storepass:env',
    'SOCKSPRING_KEY_PASSWORD', '-alias', 'upload', '-file', str(folder / 'upload-certificate.pem')], env=env, check=True)
print('Created local upload key and upload-signing.properties. Back up both securely; do not commit them.')
