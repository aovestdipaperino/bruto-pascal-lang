{ Bruto Pascal runtime: FPC's Sockets unit, for macOS and Linux. The BSD
  socket calls are the C library's; the records have each system's
  layout (macOS's start with a length byte, as FPC's do on BSD), so a
  program that fills a TInetSockAddr field by field builds on both.

  Writing to a connection the other end has closed returns -1 (EPIPE)
  rather than killing the program with SIGPIPE: fpSocket sets
  SO_NOSIGPIPE on macOS, and fpSend/fpSendTo pass MSG_NOSIGNAL on Linux. }
unit Sockets;

interface

uses UnixType, BaseUnix;

type
  TSocket = LongInt;
  TSockLen = cuint32;
  PSockLen = ^TSockLen;
{$ifdef DARWIN}
  sa_family_t = cuint8;
{$else}
  sa_family_t = cuint16;
{$endif}

  { An IPv4 address, in network byte order. }
  in_addr = record
    case Integer of
      0: (s_addr: cuint32);
      1: (s_bytes: array[1..4] of cuint8);
  end;
  TInAddr = in_addr;
  PInAddr = ^in_addr;

  sockaddr = record
{$ifdef DARWIN}
    sa_len: cuint8;
{$endif}
    sa_family: sa_family_t;
    sa_data: array[0..13] of cuint8;
  end;
  TSockAddr = sockaddr;
  PSockAddr = ^sockaddr;

  sockaddr_in = record
{$ifdef DARWIN}
    sin_len: cuint8;
{$endif}
    sin_family: sa_family_t;
    sin_port: cuint16;
    sin_addr: in_addr;
    sin_zero: array[0..7] of cuint8;
  end;
  TInetSockAddr = sockaddr_in;
  PInetSockAddr = ^sockaddr_in;

const
  AF_UNSPEC = 0;
  AF_UNIX = 1;
  AF_INET = 2;
{$ifdef DARWIN}
  AF_INET6 = 30;
{$else}
  AF_INET6 = 10;
{$endif}
  PF_UNSPEC = AF_UNSPEC;
  PF_UNIX = AF_UNIX;
  PF_INET = AF_INET;
  PF_INET6 = AF_INET6;

  SOCK_STREAM = 1;
  SOCK_DGRAM = 2;
  SOCK_RAW = 3;

  IPPROTO_IP = 0;
  IPPROTO_TCP = 6;
  IPPROTO_UDP = 17;

  INADDR_ANY = 0;
  INADDR_LOOPBACK = $7F000001;
  INADDR_BROADCAST = $FFFFFFFF;

  TCP_NODELAY = 1;

  MSG_OOB = 1;
  MSG_PEEK = 2;
  MSG_DONTWAIT = $40;
{$ifdef DARWIN}
  MSG_NOSIGNAL = 0;
{$else}
  MSG_NOSIGNAL = $4000;
{$endif}

  SHUT_RD = 0;
  SHUT_WR = 1;
  SHUT_RDWR = 2;

{$ifdef DARWIN}
  SOL_SOCKET = $FFFF;
  SO_REUSEADDR = $0004;
  SO_KEEPALIVE = $0008;
  SO_BROADCAST = $0020;
  SO_LINGER = $0080;
  SO_REUSEPORT = $0200;
  SO_SNDBUF = $1001;
  SO_RCVBUF = $1002;
  SO_SNDTIMEO = $1005;
  SO_RCVTIMEO = $1006;
  SO_ERROR = $1007;
  SO_NOSIGPIPE = $1022;
{$else}
  SOL_SOCKET = 1;
  SO_REUSEADDR = 2;
  SO_ERROR = 4;
  SO_BROADCAST = 6;
  SO_SNDBUF = 7;
  SO_RCVBUF = 8;
  SO_KEEPALIVE = 9;
  SO_LINGER = 13;
  SO_REUSEPORT = 15;
  SO_RCVTIMEO = 20;
  SO_SNDTIMEO = 21;
{$endif}

function fpSocket(Domain, XType, Protocol: cint): cint;
function fpBind(S: cint; AddrX: PSockAddr; AddrLen: TSockLen): cint;
  cdecl; external 'c' name 'bind';
function fpListen(S: cint; Backlog: cint): cint; cdecl; external 'c' name 'listen';
function fpAccept(S: cint; AddrX: PSockAddr; AddrLen: PSockLen): cint;
  cdecl; external 'c' name 'accept';
function fpConnect(S: cint; Name: PSockAddr; NameLen: TSockLen): cint;
  cdecl; external 'c' name 'connect';
function fpSend(S: cint; Msg: Pointer; Len: size_t; Flags: cint): ssize_t;
function fpRecv(S: cint; Buf: Pointer; Len: size_t; Flags: cint): ssize_t;
  cdecl; external 'c' name 'recv';
function fpSendTo(S: cint; Msg: Pointer; Len: size_t; Flags: cint; ToAddr: PSockAddr;
  ToLen: TSockLen): ssize_t;
function fpRecvFrom(S: cint; Buf: Pointer; Len: size_t; Flags: cint; From: PSockAddr;
  FromLen: PSockLen): ssize_t; cdecl; external 'c' name 'recvfrom';
function fpShutdown(S: cint; How: cint): cint; cdecl; external 'c' name 'shutdown';
function fpGetSockName(S: cint; Name: PSockAddr; NameLen: PSockLen): cint;
  cdecl; external 'c' name 'getsockname';
function fpGetPeerName(S: cint; Name: PSockAddr; NameLen: PSockLen): cint;
  cdecl; external 'c' name 'getpeername';
function fpSetSockOpt(S: cint; Level, OptName: cint; OptVal: Pointer; OptLen: TSockLen): cint;
  cdecl; external 'c' name 'setsockopt';
function fpGetSockOpt(S: cint; Level, OptName: cint; OptVal: Pointer; OptLen: PSockLen): cint;
  cdecl; external 'c' name 'getsockopt';
function CloseSocket(Sock: LongInt): LongInt;
{ The error of the last socket call that failed (errno). }
function SocketError: cint;

function htons(Host: Word): Word;
function ntohs(Net: Word): Word;
function htonl(Host: LongWord): LongWord;
function ntohl(Net: LongWord): LongWord;

{ Dotted-decimal text and addresses. A "net" address is in network byte
  order (as in a TInetSockAddr), a "host" address in the machine's. An
  address that doesn't parse is 0. }
function NetAddrToStr(Entry: in_addr): AnsiString;
function HostAddrToStr(Entry: in_addr): AnsiString;
function StrToNetAddr(IP: AnsiString): in_addr;
function StrToHostAddr(IP: AnsiString): in_addr;

implementation

function c_socket(Domain, XType, Protocol: cint): cint; cdecl; external 'c' name 'socket';
function c_send(S: cint; Msg: Pointer; Len: size_t; Flags: cint): ssize_t;
  cdecl; external 'c' name 'send';
function c_sendto(S: cint; Msg: Pointer; Len: size_t; Flags: cint; ToAddr: PSockAddr;
  ToLen: TSockLen): ssize_t; cdecl; external 'c' name 'sendto';

function fpSocket(Domain, XType, Protocol: cint): cint;
var
  S: cint;
{$ifdef DARWIN}
  One: cint;
{$endif}
begin
  S := c_socket(Domain, XType, Protocol);
{$ifdef DARWIN}
  if S >= 0 then
  begin
    One := 1;
    fpSetSockOpt(S, SOL_SOCKET, SO_NOSIGPIPE, @One, SizeOf(One));
  end;
{$endif}
  fpSocket := S;
end;

function fpSend(S: cint; Msg: Pointer; Len: size_t; Flags: cint): ssize_t;
begin
  fpSend := c_send(S, Msg, Len, Flags or MSG_NOSIGNAL);
end;

function fpSendTo(S: cint; Msg: Pointer; Len: size_t; Flags: cint; ToAddr: PSockAddr;
  ToLen: TSockLen): ssize_t;
begin
  fpSendTo := c_sendto(S, Msg, Len, Flags or MSG_NOSIGNAL, ToAddr, ToLen);
end;

function CloseSocket(Sock: LongInt): LongInt;
begin
  CloseSocket := FpClose(Sock);
end;

function SocketError: cint;
begin
  SocketError := FpGetErrno;
end;

function htons(Host: Word): Word;
begin
  htons := Word((Host shr 8) or ((Host and $FF) shl 8));
end;

function ntohs(Net: Word): Word;
begin
  ntohs := htons(Net);
end;

function htonl(Host: LongWord): LongWord;
begin
  htonl := LongWord(((Host and $FF) shl 24) or (((Host shr 8) and $FF) shl 16)
    or (((Host shr 16) and $FF) shl 8) or ((Host shr 24) and $FF));
end;

function ntohl(Net: LongWord): LongWord;
begin
  ntohl := htonl(Net);
end;

function SocketsByteToStr(B: Integer): AnsiString;
var
  S: AnsiString;
begin
  Str(B, S);
  SocketsByteToStr := S;
end;

function NetAddrToStr(Entry: in_addr): AnsiString;
var
  I: Integer;
  S: AnsiString;
begin
  S := '';
  for I := 1 to 4 do
  begin
    if I > 1 then
      S := S + '.';
    S := S + SocketsByteToStr(Entry.s_bytes[I]);
  end;
  NetAddrToStr := S;
end;

function HostAddrToStr(Entry: in_addr): AnsiString;
var
  Net: in_addr;
begin
  Net.s_addr := htonl(Entry.s_addr);
  HostAddrToStr := NetAddrToStr(Net);
end;

function StrToNetAddr(IP: AnsiString): in_addr;
var
  R: in_addr;
  Part, I, Value, Digits: Integer;
  C: Char;
  Ok: Boolean;
begin
  R.s_addr := 0;
  Part := 1;
  Value := 0;
  Digits := 0;
  Ok := Length(IP) > 0;
  I := 1;
  while Ok and (I <= Length(IP)) do
  begin
    C := IP[I];
    if (C >= '0') and (C <= '9') then
    begin
      Value := Value * 10 + Ord(C) - Ord('0');
      Digits := Digits + 1;
      Ok := (Value <= 255) and (Digits <= 3);
    end
    else if (C = '.') and (Digits > 0) and (Part < 4) then
    begin
      R.s_bytes[Part] := Value;
      Part := Part + 1;
      Value := 0;
      Digits := 0;
    end
    else
      Ok := False;
    I := I + 1;
  end;
  if Ok and (Part = 4) and (Digits > 0) then
    R.s_bytes[4] := Value
  else
    R.s_addr := 0;
  StrToNetAddr := R;
end;

function StrToHostAddr(IP: AnsiString): in_addr;
var
  R: in_addr;
begin
  R := StrToNetAddr(IP);
  R.s_addr := ntohl(R.s_addr);
  StrToHostAddr := R;
end;

end.
