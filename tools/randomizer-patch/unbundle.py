import sys, struct, os, zlib
exe, out = sys.argv[1], sys.argv[2]
sig = bytes([0x8b,0x12,0x02,0xb9,0x6a,0x61,0x20,0x38,0x72,0x7b,0x93,0x02,0x14,0xd7,0xa0,0x32,0x13,0xf5,0xb9,0xe6,0xef,0xae,0x33,0x18,0xee,0x3b,0x2d,0xce,0x24,0xb3,0x6a,0xae])
data = open(exe,'rb').read()
i = data.find(sig); hdr = struct.unpack_from('<q', data, i-8)[0]
def rstr(p):
    n=0; s=0
    while True:
        b=data[p]; p+=1; n|=(b&0x7f)<<s; s+=7
        if b<0x80: break
    return data[p:p+n].decode(), p+n
major, minor, count = struct.unpack_from('<IIi', data, hdr); p = hdr+12
bid, p = rstr(p)
if major >= 2: p += 8*4 + 8
print('bundle v', major, minor, count, 'files')
for _ in range(count):
    off, size = struct.unpack_from('<qq', data, p); p += 16
    csize = 0
    if major >= 6: csize = struct.unpack_from('<q', data, p)[0]; p += 8
    typ = data[p]; p += 1
    path, p = rstr(p)
    if not path.lower().endswith('.dll') or path.lower().startswith(('system.','microsoft.','presentation','windows','mscorlib','netstandard','wpf','direct','uiautomation','reachframework')): continue
    blob = data[off:off+(csize or size)]
    if csize: blob = zlib.decompress(blob, -15)
    dst = os.path.join(out, path); os.makedirs(os.path.dirname(dst) or out, exist_ok=True)
    open(dst,'wb').write(blob); print(path, len(blob))
