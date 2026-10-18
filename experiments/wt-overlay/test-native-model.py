"""Run C ABI regressions against the compiled production-buffer observer DLL."""
import argparse
import ctypes as c
import hashlib
import json
import os
from pathlib import Path


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--directory',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    assert not args.output.exists()
    folder=args.directory.resolve(strict=True)
    dlls=list(folder.rglob('Blueberry.NativeModel.dll'))
    assert len(dlls)==1,'Native model DLL missing or ambiguous'
    library_path=dlls[0]
    with os.add_dll_directory(str(library_path.parent)):
        library=c.CDLL(str(library_path))
    response=c.CFUNCTYPE(None,c.c_void_p,c.POINTER(c.c_wchar),c.c_uint32)
    library.bb_native_model_create.argtypes=[c.c_uint16,c.c_uint16,c.c_uint16,response,c.c_void_p]
    library.bb_native_model_create.restype=c.c_void_p
    library.bb_native_model_destroy.argtypes=[c.c_void_p]
    library.bb_native_model_destroy.restype=None
    library.bb_native_model_write.argtypes=[c.c_void_p,c.c_char_p,c.c_uint32]
    library.bb_native_model_write.restype=c.c_int32
    library.bb_native_model_resize.argtypes=[c.c_void_p,c.c_uint16,c.c_uint16]
    library.bb_native_model_resize.restype=c.c_int32
    library.bb_native_model_snapshot.argtypes=[c.c_void_p,c.POINTER(c.c_wchar),c.c_uint32,c.POINTER(c.c_uint32)]
    library.bb_native_model_snapshot.restype=c.c_int32
    library.bb_native_model_history.argtypes=library.bb_native_model_snapshot.argtypes
    library.bb_native_model_history.restype=c.c_int32
    library.bb_native_model_cell_colors.argtypes=[c.c_void_p,c.c_uint32,c.c_uint32,c.POINTER(c.c_uint32)]
    library.bb_native_model_cell_colors.restype=c.c_int32
    replies=[]
    @response
    def reply(_,text,length): replies.append(c.wstring_at(text,length))
    handles=[]
    result=dict(schema=1,diagnostic_only=True,gui_renderer=False,pixel_visibility_measured=False,
                dll_sha256=hashlib.sha256(library_path.read_bytes()).hexdigest(),tests=[],passed=False)

    def create():
        handle=library.bb_native_model_create(12,40,100,reply,None)
        assert handle,'Native model construction failed'
        handles.append(handle)
        return handle
    def write(handle,value):
        data=value.encode('utf-8') if isinstance(value,str) else value
        status=library.bb_native_model_write(handle,data,len(data))
        assert status>=0,f'Native write failed: {status:#x}'
    def snapshot(handle,history=False):
        read=library.bb_native_model_history if history else library.bb_native_model_snapshot
        needed=c.c_uint32()
        assert read(handle,None,0,c.byref(needed))==-2147024774
        assert 1<=needed.value<=(32 if history else 2)*1024*1024
        buffer=c.create_unicode_buffer(needed.value)
        assert read(handle,buffer,len(buffer),c.byref(needed))==0
        return c.wstring_at(buffer,needed.value-1)

    try:
        result['abi_version']=library.bb_native_model_version()
        result['interface_version']=library.bb_native_model_overlay_version()
        assert result['abi_version']==1 and result['interface_version']==4
        plain,overlaid=create(),create()
        history='\x1b[38;2;12;34;56m\x1b]8;id=old;https://example.invalid/history\x1b\\历史🚀é'+'w'*90+'\x1b]8;;\x1b\\\x1b[0m\r\n$ '
        write(plain,history); write(overlaid,history)
        assert snapshot(plain)==snapshot(overlaid)
        for generation,(rows,columns) in enumerate(((12,20),(14,60),(12,30)),1):
            text='\x1b[38;2;120;45;67mmenu🚀\x1b[0m'
            hex_text=''.join(f'{int.from_bytes(text.encode("utf-16-le")[i:i+2],"little"):04x}' for i in range(0,len(text.encode('utf-16-le')),2))
            write(overlaid,f'\x1b]7775;frame;{generation};0;1;16;{hex_text}\x07')
            assert 'menu' in snapshot(overlaid)
            for handle in (plain,overlaid): assert library.bb_native_model_resize(handle,rows,columns)==0
            assert snapshot(plain)==snapshot(overlaid),'Native resize restored different text'
        result['tests'].append('overlay/native-reflow round trips across three resizes')
        scrollback=create()
        write(scrollback,'before\r\n'*15+'\x1b[38;2;12;34;56mRGB-历史🚀é\x1b[0m\r\n'+'after\r\n'*15)
        assert 'RGB-' not in snapshot(scrollback),'Fixture must be outside the viewport'
        for rows,columns in ((8,20),(20,60),(12,40)):
            assert library.bb_native_model_resize(scrollback,rows,columns)==0
            lines=snapshot(scrollback,history=True).splitlines()
            matches=[(y,line.index('RGB-')) for y,line in enumerate(lines) if 'RGB-历史🚀é' in line]
            assert len(matches)==1,'Scrollback Unicode sentinel missing after native reflow'
            colors=(c.c_uint32*4)()
            assert library.bb_native_model_cell_colors(scrollback,*matches[0],colors)==0
            assert list(colors)[:2]==[3,12|(34<<8)|(56<<16)],'History lost original RGB color'
        colors=(c.c_uint32*4)()
        assert library.bb_native_model_cell_colors(scrollback,0xffffffff,0,colors)<0
        result['tests'].append('off-viewport Unicode and original RGB across three native resizes')
        split=create()
        for byte in '中🚀é'.encode('utf-8'): write(split,bytes([byte]))
        assert '中🚀é' in snapshot(split)
        result['tests'].append('split UTF-8 scalars and combining text')
        invalid=create()
        assert library.bb_native_model_write(invalid,b'\xff',1)<0
        assert library.bb_native_model_write(invalid,b'x',1024*1024+1)<0
        result['tests'].append('malformed UTF-8 and oversized writes rejected')
        write(plain,'\x1b]7775;query;123\x07')
        assert replies[-1]=='\x1b]7775;prototype;1;123;fixed-menu\x07'
        result['tests'].append('owned response callback')
        result['passed']=True
    except Exception as error:
        result['error']=str(error)
    finally:
        for handle in reversed(handles): library.bb_native_model_destroy(handle)
        args.output.write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(result))
    return 0 if result['passed'] else 1


if __name__=='__main__': raise SystemExit(main())
