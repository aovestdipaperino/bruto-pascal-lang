{ Bruto Pascal runtime: network access as Turbo Pascal objects, built on
  the Sockets and NetDB units.

    TSocketBase   a socket: handle, last error, Close, timeouts
    TTcpClient    a TCP connection: Connect by host name, Send/Receive,
                  SendString, line-by-line ReadLine, ReadAll
    TTcpServer    a listening socket; Accept returns a new PTcpClient
    TUdpSocket    datagrams: SendTo / ReceiveFrom
    THttpClient   plain HTTP (no TLS): Get returns the status code and
                  fills Body and Headers

  Constructors that can't do their job Fail, so New(P, Connect(...))
  gives nil, and a static object's constructor returns False. }
unit NetObjects;

interface

uses UnixType, Sockets, NetDB;

type
  PSocketBase = ^TSocketBase;
  TSocketBase = object
    { The socket, or -1 when closed. }
    Handle: LongInt;
    { errno of the last call that failed, 0 if none did. }
    LastError: LongInt;
    constructor Init;
    destructor Done; virtual;
    function IsOpen: Boolean;
    procedure Close; virtual;
    { Make Receive (and Send) give up after Milliseconds (0: never). }
    function SetTimeout(Milliseconds: LongInt): Boolean;
  end;

  PTcpClient = ^TTcpClient;
  TTcpClient = object(TSocketBase)
    { Bytes received but not yet returned by ReadLine. }
    Pending: AnsiString;
    { Connect to Host (a name or a dotted address) on Port. }
    constructor Connect(const Host: AnsiString; Port: Word);
    { Wrap a socket already connected (as TTcpServer.Accept does). }
    constructor FromHandle(AHandle: LongInt);
    { Send up to Count bytes of Buf; the number sent, or -1. }
    function Send(const Buf; Count: LongInt): LongInt;
    { Receive up to Count bytes into Buf; the number received, 0 when the
      other end has closed, -1 on error (or timeout). }
    function Receive(var Buf; Count: LongInt): LongInt;
    { Send all of S. }
    function SendString(const S: AnsiString): Boolean;
    { The next line (without its CR LF or LF); False at the end. }
    function ReadLine(var S: AnsiString): Boolean;
    { Everything until the other end closes. }
    function ReadAll: AnsiString;
    { The other end's address, as a.b.c.d. }
    function RemoteAddress: AnsiString;
  end;

  PTcpServer = ^TTcpServer;
  TTcpServer = object(TSocketBase)
    { Listen on Port (0: any free port, see LocalPort) on all addresses. }
    constructor Listen(APort: Word; Backlog: LongInt);
    { The next connection, waiting for one; nil on error. }
    function Accept: PTcpClient;
    { The port listened on. }
    function LocalPort: Word;
  end;

  PUdpSocket = ^TUdpSocket;
  TUdpSocket = object(TSocketBase)
    { A datagram socket bound to Port (0: any). }
    constructor Init(APort: Word);
    function SendTo(const Host: AnsiString; Port: Word; const Data: AnsiString): Boolean;
    { The next datagram; False on error (or timeout). }
    function ReceiveFrom(var Data: AnsiString; var FromHost: AnsiString;
      var FromPort: Word): Boolean;
    function LocalPort: Word;
  end;

  PHttpClient = ^THttpClient;
  THttpClient = object
    { Of the last request: the response body, the header lines (CR LF
      separated, status line excluded), and the error text when Get
      returns 0. }
    Body: AnsiString;
    Headers: AnsiString;
    ErrorText: AnsiString;
    { Receive timeout in milliseconds (0: none). }
    Timeout: LongInt;
    constructor Init;
    destructor Done; virtual;
    { GET an http:// URL: the status code (200...), or 0 on failure. }
    function Get(const Url: AnsiString): LongInt;
    { The value of response header Name (case-insensitive), or ''. }
    function Header(const Name: AnsiString): AnsiString;
  end;

{ Resolve Host (a name or a.b.c.d) into a socket address for Port. }
function ResolveAddress(const Host: AnsiString; Port: Word; var Addr: TInetSockAddr): Boolean;

implementation

const
  { The receive buffer's size. }
  ChunkSize = 4096;

function ResolveAddress(const Host: AnsiString; Port: Word; var Addr: TInetSockAddr): Boolean;
var
  H: THostEntry;
  A: in_addr;
begin
  ResolveAddress := False;
  FillChar(Addr, SizeOf(Addr), 0);
{$ifdef DARWIN}
  Addr.sin_len := SizeOf(Addr);
{$endif}
  Addr.sin_family := AF_INET;
  Addr.sin_port := htons(Port);
  A := StrToNetAddr(Host);
  if A.s_addr = 0 then
  begin
    if not ResolveHostByName(Host, H) then
      Exit;
    A.s_addr := htonl(H.Addr.s_addr);
  end;
  Addr.sin_addr := A;
  ResolveAddress := True;
end;

{ ── TSocketBase ──────────────────────────────────────────── }

constructor TSocketBase.Init;
begin
  Handle := -1;
  LastError := 0;
end;

destructor TSocketBase.Done;
begin
  Close;
end;

function TSocketBase.IsOpen: Boolean;
begin
  IsOpen := Handle >= 0;
end;

procedure TSocketBase.Close;
begin
  if Handle >= 0 then
    CloseSocket(Handle);
  Handle := -1;
end;

function TSocketBase.SetTimeout(Milliseconds: LongInt): Boolean;
var
  Tv: TTimeVal;
begin
  Tv.tv_sec := Milliseconds div 1000;
  Tv.tv_usec := (Milliseconds mod 1000) * 1000;
  SetTimeout := (fpSetSockOpt(Handle, SOL_SOCKET, SO_RCVTIMEO, @Tv, SizeOf(Tv)) = 0)
    and (fpSetSockOpt(Handle, SOL_SOCKET, SO_SNDTIMEO, @Tv, SizeOf(Tv)) = 0);
end;

{ ── TTcpClient ───────────────────────────────────────────── }

constructor TTcpClient.Connect(const Host: AnsiString; Port: Word);
var
  Addr: TInetSockAddr;
begin
  if not inherited Init then
    Fail;
  Pending := '';
  if not ResolveAddress(Host, Port, Addr) then
    Fail;
  Handle := fpSocket(AF_INET, SOCK_STREAM, 0);
  if Handle < 0 then
    Fail;
  if fpConnect(Handle, @Addr, SizeOf(Addr)) <> 0 then
  begin
    LastError := SocketError;
    Close;
    Fail;
  end;
end;

constructor TTcpClient.FromHandle(AHandle: LongInt);
begin
  if not inherited Init then
    Fail;
  Pending := '';
  Handle := AHandle;
end;

function TTcpClient.Send(const Buf; Count: LongInt): LongInt;
var
  N: LongInt;
begin
  N := fpSend(Handle, @Buf, Count, 0);
  if N < 0 then
    LastError := SocketError;
  Send := N;
end;

function TTcpClient.Receive(var Buf; Count: LongInt): LongInt;
var
  N: LongInt;
begin
  N := fpRecv(Handle, @Buf, Count, 0);
  if N < 0 then
    LastError := SocketError;
  Receive := N;
end;

function TTcpClient.SendString(const S: AnsiString): Boolean;
var
  Done, N: LongInt;
  P: PChar;
begin
  SendString := True;
  if Length(S) = 0 then
    Exit;
  P := PChar(S);
  Done := 0;
  while Done < Length(S) do
  begin
    N := fpSend(Handle, P + Done, Length(S) - Done, 0);
    if N <= 0 then
    begin
      LastError := SocketError;
      SendString := False;
      Exit;
    end;
    Done := Done + N;
  end;
end;

function TTcpClient.ReadLine(var S: AnsiString): Boolean;
var
  Buf: array[0..ChunkSize - 1] of Char;
  N, I: LongInt;
begin
  ReadLine := False;
  repeat
    I := Pos(#10, Pending);
    if I > 0 then
    begin
      S := Copy(Pending, 1, I - 1);
      if (Length(S) > 0) and (S[Length(S)] = #13) then
        S := Copy(S, 1, Length(S) - 1);
      Pending := Copy(Pending, I + 1, Length(Pending));
      ReadLine := True;
      Exit;
    end;
    N := Receive(Buf, SizeOf(Buf));
    if N > 0 then
    begin
      SetLength(S, N);
      Move(Buf, S[1], N);
      Pending := Pending + S;
    end;
  until N <= 0;
  { The end: the last, unterminated line, if any. }
  if Length(Pending) > 0 then
  begin
    S := Pending;
    Pending := '';
    ReadLine := True;
  end;
end;

function TTcpClient.ReadAll: AnsiString;
var
  Buf: array[0..ChunkSize - 1] of Char;
  N: LongInt;
  R, Part: AnsiString;
begin
  R := Pending;
  Pending := '';
  repeat
    N := Receive(Buf, SizeOf(Buf));
    if N > 0 then
    begin
      SetLength(Part, N);
      Move(Buf, Part[1], N);
      R := R + Part;
    end;
  until N <= 0;
  ReadAll := R;
end;

function TTcpClient.RemoteAddress: AnsiString;
var
  Addr: TInetSockAddr;
  Len: TSockLen;
begin
  Len := SizeOf(Addr);
  if fpGetPeerName(Handle, @Addr, @Len) = 0 then
    RemoteAddress := NetAddrToStr(Addr.sin_addr)
  else
    RemoteAddress := '';
end;

{ ── TTcpServer ───────────────────────────────────────────── }

constructor TTcpServer.Listen(APort: Word; Backlog: LongInt);
var
  Addr: TInetSockAddr;
  One: LongInt;
begin
  if not inherited Init then
    Fail;
  Handle := fpSocket(AF_INET, SOCK_STREAM, 0);
  if Handle < 0 then
    Fail;
  One := 1;
  fpSetSockOpt(Handle, SOL_SOCKET, SO_REUSEADDR, @One, SizeOf(One));
  FillChar(Addr, SizeOf(Addr), 0);
{$ifdef DARWIN}
  Addr.sin_len := SizeOf(Addr);
{$endif}
  Addr.sin_family := AF_INET;
  Addr.sin_port := htons(APort);
  Addr.sin_addr.s_addr := INADDR_ANY;
  if (fpBind(Handle, @Addr, SizeOf(Addr)) <> 0) or (fpListen(Handle, Backlog) <> 0) then
  begin
    LastError := SocketError;
    Close;
    Fail;
  end;
end;

function TTcpServer.Accept: PTcpClient;
var
  Addr: TInetSockAddr;
  Len: TSockLen;
  S: LongInt;
begin
  Accept := nil;
  Len := SizeOf(Addr);
  S := fpAccept(Handle, @Addr, @Len);
  if S < 0 then
  begin
    LastError := SocketError;
    Exit;
  end;
  Accept := New(PTcpClient, FromHandle(S));
end;

function TTcpServer.LocalPort: Word;
var
  Addr: TInetSockAddr;
  Len: TSockLen;
begin
  Len := SizeOf(Addr);
  if fpGetSockName(Handle, @Addr, @Len) = 0 then
    LocalPort := ntohs(Addr.sin_port)
  else
    LocalPort := 0;
end;

{ ── TUdpSocket ───────────────────────────────────────────── }

constructor TUdpSocket.Init(APort: Word);
var
  Addr: TInetSockAddr;
begin
  if not inherited Init then
    Fail;
  Handle := fpSocket(AF_INET, SOCK_DGRAM, 0);
  if Handle < 0 then
    Fail;
  FillChar(Addr, SizeOf(Addr), 0);
{$ifdef DARWIN}
  Addr.sin_len := SizeOf(Addr);
{$endif}
  Addr.sin_family := AF_INET;
  Addr.sin_port := htons(APort);
  Addr.sin_addr.s_addr := INADDR_ANY;
  if fpBind(Handle, @Addr, SizeOf(Addr)) <> 0 then
  begin
    LastError := SocketError;
    Close;
    Fail;
  end;
end;

function TUdpSocket.SendTo(const Host: AnsiString; Port: Word; const Data: AnsiString): Boolean;
var
  Addr: TInetSockAddr;
begin
  SendTo := False;
  if not ResolveAddress(Host, Port, Addr) then
    Exit;
  if fpSendTo(Handle, PChar(Data), Length(Data), 0, @Addr, SizeOf(Addr)) < 0 then
  begin
    LastError := SocketError;
    Exit;
  end;
  SendTo := True;
end;

function TUdpSocket.ReceiveFrom(var Data: AnsiString; var FromHost: AnsiString;
  var FromPort: Word): Boolean;
var
  Buf: array[0..65535] of Char;
  Addr: TInetSockAddr;
  Len: TSockLen;
  N: LongInt;
begin
  ReceiveFrom := False;
  Len := SizeOf(Addr);
  N := fpRecvFrom(Handle, @Buf, SizeOf(Buf), 0, @Addr, @Len);
  if N < 0 then
  begin
    LastError := SocketError;
    Exit;
  end;
  SetLength(Data, N);
  if N > 0 then
    Move(Buf, Data[1], N);
  FromHost := NetAddrToStr(Addr.sin_addr);
  FromPort := ntohs(Addr.sin_port);
  ReceiveFrom := True;
end;

function TUdpSocket.LocalPort: Word;
var
  Addr: TInetSockAddr;
  Len: TSockLen;
begin
  Len := SizeOf(Addr);
  if fpGetSockName(Handle, @Addr, @Len) = 0 then
    LocalPort := ntohs(Addr.sin_port)
  else
    LocalPort := 0;
end;

{ ── THttpClient ──────────────────────────────────────────── }

constructor THttpClient.Init;
begin
  Body := '';
  Headers := '';
  ErrorText := '';
  Timeout := 30000;
end;

destructor THttpClient.Done;
begin
  Body := '';
  Headers := '';
  ErrorText := '';
end;

function LowerText(const S: AnsiString): AnsiString;
var
  I: LongInt;
  R: AnsiString;
begin
  R := S;
  for I := 1 to Length(R) do
    if (R[I] >= 'A') and (R[I] <= 'Z') then
      R[I] := Chr(Ord(R[I]) + 32);
  LowerText := R;
end;

function TrimText(const S: AnsiString): AnsiString;
var
  First, Last: LongInt;
begin
  First := 1;
  Last := Length(S);
  while (First <= Last) and (S[First] <= ' ') do
    First := First + 1;
  while (Last >= First) and (S[Last] <= ' ') do
    Last := Last - 1;
  TrimText := Copy(S, First, Last - First + 1);
end;

function THttpClient.Header(const Name: AnsiString): AnsiString;
var
  Rest, Line, Key: AnsiString;
  I, Colon: LongInt;
begin
  Header := '';
  Key := LowerText(Name);
  Rest := Headers;
  while Length(Rest) > 0 do
  begin
    I := Pos(#13#10, Rest);
    if I = 0 then
    begin
      Line := Rest;
      Rest := '';
    end
    else
    begin
      Line := Copy(Rest, 1, I - 1);
      Rest := Copy(Rest, I + 2, Length(Rest));
    end;
    Colon := Pos(':', Line);
    if (Colon > 0) and (LowerText(TrimText(Copy(Line, 1, Colon - 1))) = Key) then
    begin
      Header := TrimText(Copy(Line, Colon + 1, Length(Line)));
      Exit;
    end;
  end;
end;

{ A chunked body ("Transfer-Encoding: chunked") put back together. }
function Unchunk(const Raw: AnsiString): AnsiString;
var
  R, SizeText: AnsiString;
  P, LineEnd, Size, I, Code: LongInt;
begin
  R := '';
  P := 1;
  while P <= Length(Raw) do
  begin
    LineEnd := Pos(#13#10, Copy(Raw, P, Length(Raw)));
    if LineEnd = 0 then
      Break;
    SizeText := Copy(Raw, P, LineEnd - 1);
    I := Pos(';', SizeText);
    if I > 0 then
      SizeText := Copy(SizeText, 1, I - 1);
    Val('$' + TrimText(SizeText), Size, Code);
    if (Code <> 0) or (Size <= 0) then
      Break;
    P := P + LineEnd + 1;
    R := R + Copy(Raw, P, Size);
    P := P + Size + 2;
  end;
  Unchunk := R;
end;

function THttpClient.Get(const Url: AnsiString): LongInt;
var
  Rest, Host, Path, Line, Raw: AnsiString;
  Port: Word;
  I, Code, Status: LongInt;
  Conn: TTcpClient;
begin
  Get := 0;
  Body := '';
  Headers := '';
  ErrorText := '';
  if LowerText(Copy(Url, 1, 7)) <> 'http://' then
  begin
    ErrorText := 'only http:// URLs are supported';
    Exit;
  end;
  Rest := Copy(Url, 8, Length(Url));
  I := Pos('/', Rest);
  if I > 0 then
  begin
    Host := Copy(Rest, 1, I - 1);
    Path := Copy(Rest, I, Length(Rest));
  end
  else
  begin
    Host := Rest;
    Path := '/';
  end;
  Port := 80;
  I := Pos(':', Host);
  if I > 0 then
  begin
    Val(Copy(Host, I + 1, Length(Host)), Port, Code);
    Host := Copy(Host, 1, I - 1);
  end;
  if not Conn.Connect(Host, Port) then
  begin
    ErrorText := 'cannot connect to ' + Host;
    Exit;
  end;
  if Timeout > 0 then
    Conn.SetTimeout(Timeout);
  if not Conn.SendString('GET ' + Path + ' HTTP/1.1'#13#10 + 'Host: ' + Host + #13#10 +
    'User-Agent: Bruto-Pascal'#13#10 + 'Connection: close'#13#10#13#10) then
  begin
    ErrorText := 'cannot send the request';
    Conn.Done;
    Exit;
  end;
  { Status line: HTTP/1.1 200 OK }
  if not Conn.ReadLine(Line) then
  begin
    ErrorText := 'no response';
    Conn.Done;
    Exit;
  end;
  I := Pos(' ', Line);
  Val(Copy(Line, I + 1, 3), Status, Code);
  if (I = 0) or (Code <> 0) then
  begin
    ErrorText := 'bad status line: ' + Line;
    Conn.Done;
    Exit;
  end;
  while Conn.ReadLine(Line) and (Length(Line) > 0) do
    Headers := Headers + Line + #13#10;
  Raw := Conn.ReadAll;
  Conn.Done;
  if Pos('chunked', LowerText(Header('Transfer-Encoding'))) > 0 then
    Body := Unchunk(Raw)
  else
    Body := Raw;
  Get := Status;
end;

end.
