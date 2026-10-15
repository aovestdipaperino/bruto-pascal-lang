{ Bruto Pascal runtime: the part of FPC's BaseUnix unit Free Vision uses,
  bound straight to the C library. }
unit BaseUnix;

interface

uses UnixType;

function FpGetEnv(Name: PChar): PChar; cdecl; external 'c' name 'getenv';
function FpNanoSleep(Req, Rem: PTimeSpec): cint; cdecl; external 'c' name 'nanosleep';
function FpRead(Fd: cint; Buf: Pointer; Count: size_t): ssize_t; cdecl; external 'c' name 'read';
function FpWrite(Fd: cint; Buf: Pointer; Count: size_t): ssize_t; cdecl; external 'c' name 'write';
function FpGetPid: cint; cdecl; external 'c' name 'getpid';
function FpIOCtl(Fd: cint; Request: culong): cint; cdecl; varargs; external 'c' name 'ioctl';

implementation

end.
