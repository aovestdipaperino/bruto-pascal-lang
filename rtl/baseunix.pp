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
function FpClose(Fd: cint): cint; cdecl; external 'c' name 'close';
{ The C library's errno: the error of the last call that failed. }
function FpGetErrno: cint;

implementation

{$ifdef DARWIN}
function c_errno_location: pcint; cdecl; external 'c' name '__error';
{$else}
function c_errno_location: pcint; cdecl; external 'c' name '__errno_location';
{$endif}

function FpGetErrno: cint;
var
  P: pcint;
begin
  P := c_errno_location();
  FpGetErrno := P^;
end;

end.
