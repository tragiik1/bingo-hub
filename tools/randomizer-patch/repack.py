# Replace one file inside a .NET single-file bundle: the new bytes go at the end (4 KiB aligned),
# followed by a copy of the manifest with that entry's offset/size updated, and the header pointer
# in the host is moved to the new manifest.
import sys, struct
exe, name, newfile, out = sys.argv[1:5]
data = bytearray(open(exe,'rb').read()); new = open(newfile,'rb').read()
sig = bytes([0x8b,0x12,0x02,0xb9,0x6a,0x61,0x20,0x38,0x72,0x7b,0x93,0x02,0x14,0xd7,0xa0,0x32,0x13,0xf5,0xb9,0xe6,0xef,0xae,0x33,0x18,0xee,0x3b,0x2d,0xce,0x24,0xb3,0x6a,0xae])
i = data.find(sig); assert data.find(sig, i+1) == -1
hdr = struct.unpack_from('<q', data, i-8)[0]
def rstr(p):
    n=0; s=0
    while True:
        b=data[p]; p+=1; n|=(b&0x7f)<<s; s+=7
        if b<0x80: break
    return data[p:p+n].decode(), p+n
major, minor, count = struct.unpack_from('<IIi', data, hdr); assert major == 6
p = hdr+12; _, p = rstr(p); p += 40
target = None
for _ in range(count):
    ep = p; off, size, cs = struct.unpack_from('<qqq', data, p); p += 25
    path, p = rstr(p)
    if path == name: target = ep; assert cs == 0
manifest_end = p
assert target is not None
manifest = bytearray(data[hdr:manifest_end]); tail = bytes(data[manifest_end:])
body = bytearray(data[:hdr])
pad = (-len(body)) % 4096; body += b'\0' * pad
newoff = len(body); body += new
newhdr = len(body)
struct.pack_into('<qq', manifest, target - hdr, newoff, len(new))
# The bundle ID names the folder the exe unpacks itself to (%TEMP%\.net\<app>\<id>), and an
# existing folder is reused. Keeping the old ID would keep running the old, unpacked dll, so the
# new build gets its own ID (same length, so the manifest layout doesn't move).
import hashlib, base64
old_id, _ = rstr(hdr+12); assert len(old_id) < 128
new_id = base64.urlsafe_b64encode(hashlib.sha256(new).digest()).decode()[:len(old_id)]
manifest[13:13+len(old_id)] = new_id.encode()
body += manifest + tail
struct.pack_into('<q', body, i-8, newhdr)
open(out,'wb').write(body)
print('dll at', newoff, 'size', len(new), 'manifest at', newhdr, 'tail bytes', len(tail))
