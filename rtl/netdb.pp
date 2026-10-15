{ Bruto Pascal runtime: host name lookup, the core of FPC's netdb unit,
  through the C library's getaddrinfo (so /etc/hosts and DNS both work).
  IPv4 only. }
unit NetDB;

interface

uses UnixType, Sockets;

type
  { An IPv4 address in host byte order, as FPC's netdb has it. }
  THostAddr = in_addr;

  THostEntry = record
    Name: AnsiString;
    Addr: THostAddr;
    Aliases: AnsiString;
  end;

{ Look HostName up; on success, H holds its canonical name and its
  first IPv4 address. A dotted-decimal HostName is taken as it is. }
function ResolveHostByName(HostName: AnsiString; var H: THostEntry): Boolean;

implementation

type
  { struct addrinfo: macOS puts ai_canonname before ai_addr, Linux
    after. }
  PAddrInfo = ^TAddrInfo;
  TAddrInfo = record
    ai_flags: cint;
    ai_family: cint;
    ai_socktype: cint;
    ai_protocol: cint;
    ai_addrlen: TSockLen;
{$ifdef DARWIN}
    ai_canonname: PChar;
    ai_addr: PSockAddr;
{$else}
    ai_addr: PSockAddr;
    ai_canonname: PChar;
{$endif}
    ai_next: PAddrInfo;
  end;
  PPAddrInfo = ^PAddrInfo;

const
  AI_CANONNAME = 2;

function c_getaddrinfo(Node, Service: PChar; Hints: PAddrInfo; Res: PPAddrInfo): cint;
  cdecl; external 'c' name 'getaddrinfo';
procedure c_freeaddrinfo(Res: PAddrInfo); cdecl; external 'c' name 'freeaddrinfo';

function ResolveHostByName(HostName: AnsiString; var H: THostEntry): Boolean;
var
  Hints: TAddrInfo;
  Res, Info: PAddrInfo;
  Name: AnsiString;
begin
  ResolveHostByName := False;
  FillChar(Hints, SizeOf(Hints), 0);
  Hints.ai_family := AF_INET;
  Hints.ai_socktype := SOCK_STREAM;
  Hints.ai_flags := AI_CANONNAME;
  Res := nil;
  if c_getaddrinfo(PChar(HostName), nil, @Hints, @Res) <> 0 then
    Exit;
  Info := Res;
  while (Info <> nil) and (Info^.ai_family <> AF_INET) do
    Info := Info^.ai_next;
  if Info <> nil then
  begin
    if Res^.ai_canonname <> nil then
      Name := Res^.ai_canonname
    else
      Name := HostName;
    H.Name := Name;
    H.Aliases := '';
    H.Addr.s_addr := ntohl(PInetSockAddr(Info^.ai_addr)^.sin_addr.s_addr);
    ResolveHostByName := True;
  end;
  c_freeaddrinfo(Res);
end;

end.
