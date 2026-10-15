{ Bruto Pascal runtime: FPC's ssockets unit, socket streams with
  pluggable handlers (sslsockets' TSSLSocketHandler adds TLS): a client
  TInetSocket, and servers that accept connections (TInetServer).
  IPv4, macOS and Linux. }
unit ssockets;

interface

uses SysUtils, Classes, UnixType, Sockets, NetDB;

type
  TSocketErrorType = (seHostNotFound, seCreationFailed, seBindFailed, seListenFailed,
    seConnectFailed, seConnectTimeOut, seAcceptFailed, seAcceptWouldBlock, seIOTimeOut);

  ESocketError = class(Exception)
  private
    FCode: TSocketErrorType;
  public
    constructor Create(ACode: TSocketErrorType; const Msg: string);
    property Code: TSocketErrorType read FCode;
  end;

  TSocketState = (sosCanread, sosCanWrite, sosException);
  TSocketStates = set of TSocketState;

  TSocketStream = class;

  { The socket operations a stream uses; TLS handlers replace them. }
  TSocketHandler = class(TObject)
  private
    FSocket: TSocketStream;
  protected
    FLastError: Integer;
    procedure SetSocket(const AStream: TSocketStream); virtual;
    procedure CheckSocket;
  public
    constructor Create; virtual;
    { Called once the connection is made (client) or accepted (server). }
    function Connect: Boolean; virtual;
    function Accept: Boolean; virtual;
    function Close: Boolean; virtual;
    function Shutdown(BiDirectional: Boolean): Boolean; virtual;
    function Select(aCheck: TSocketStates; TimeOut: Integer): TSocketStates; virtual;
    function CanRead(TimeOut: Integer): Boolean; virtual;
    function Recv(const Buffer; Count: Integer): Integer; virtual;
    function Send(const Buffer; Count: Integer): Integer; virtual;
    function BytesAvailable: Integer; virtual;
    function GetLastErrorDescription: string; virtual;
    { A handler like this one for a connection a server accepts (a TLS
      handler passes on its certificate). }
    function CreateClientHandler: TSocketHandler; virtual;
    property Socket: TSocketStream read FSocket;
    property LastError: Integer read FLastError;
  end;
  TSocketHandlerClass = class of TSocketHandler;

  TSocketStream = class(THandleStream)
  private
    FClosed: Boolean;
    FPeerClosed: Boolean;
    FHandler: TSocketHandler;
    FIOTimeout: Integer;
    FConnectTimeout: Integer;
    FReadFlags: Integer;
    FWriteFlags: Integer;
    FOnClose: TNotifyEvent;
    procedure SetIOTimeout(AValue: Integer);
    function GetLastError: Integer;
    function GetLocalAddress: TSockAddr;
    function GetRemoteAddress: TSockAddr;
  protected
    procedure DoOnClose; virtual;
  public
    constructor Create(AHandle: LongInt; AHandler: TSocketHandler = nil); virtual;
    destructor Destroy; override;
    procedure Close;
    function Seek(const Offset: Int64; Origin: TSeekOrigin): Int64; override;
    function Select(aCheck: TSocketStates; TimeOut: Integer): TSocketStates;
    function CanRead(TimeOut: Integer): Boolean; virtual;
    function Read(var Buffer; Count: LongInt): LongInt; override;
    function Write(const Buffer; Count: LongInt): LongInt; override;
    property LocalAddress: TSockAddr read GetLocalAddress;
    property RemoteAddress: TSockAddr read GetRemoteAddress;
    property LastError: Integer read GetLastError;
    property ReadFlags: Integer read FReadFlags write FReadFlags;
    property WriteFlags: Integer read FWriteFlags write FWriteFlags;
    { Milliseconds a read or write may wait (0: no limit). }
    property IOTimeout: Integer read FIOTimeout write SetIOTimeout;
    property ConnectTimeout: Integer read FConnectTimeout write FConnectTimeout;
    property OnClose: TNotifyEvent read FOnClose write FOnClose;
    property Handler: TSocketHandler read FHandler;
    property Closed: Boolean read FClosed;
    property PeerClosed: Boolean read FPeerClosed;
  end;

  TInetSocket = class(TSocketStream)
  private
    FHost: string;
    FPort: Word;
  public
    { Connect to AHost (a name or a dotted address) on APort. }
    constructor Create(const AHost: string; APort: Word; AHandler: TSocketHandler = nil); overload;
    constructor Create(const AHost: string; APort: Word; aConnectTimeout: Integer;
      AHandler: TSocketHandler = nil); overload;
    procedure Connect; virtual;
    property Host: string read FHost;
    property Port: Word read FPort;
  end;

  TConnectEvent = procedure(Sender: TObject; Data: TSocketStream) of object;
  TConnectQuery = procedure(Sender: TObject; ASocket: LongInt; var Allow: Boolean) of object;
  TGetClientSocketHandlerEvent = procedure(Sender: TObject; out AHandler: TSocketHandler) of object;

  TSocketServer = class(TObject)
  private
    FOnConnect: TConnectEvent;
    FOnConnectQuery: TConnectQuery;
    FOnIdle: TNotifyEvent;
    FOnCreateClientSocketHandler: TGetClientSocketHandlerEvent;
    FAccepting: Boolean;
    FMaxConnections: LongInt;
    FQueueSize: Integer;
    FHandler: TSocketHandler;
    FAcceptIdleTimeOut: Integer;
  protected
    FSocket: LongInt;
    FBound: Boolean;
    procedure Bind; virtual; abstract;
    function Accept: LongInt; virtual; abstract;
    function SockToStream(ASocket: LongInt): TSocketStream; virtual; abstract;
    function GetClientSocketHandler(ASocket: LongInt): TSocketHandler; virtual;
    procedure DoConnect(ASocket: TSocketStream); virtual;
    function DoConnectQuery(ASocket: LongInt): Boolean; virtual;
  public
    constructor Create(ASocket: LongInt; AHandler: TSocketHandler);
    destructor Destroy; override;
    procedure Listen;
    { Accept connections, calling OnConnect for each, until
      StopAccepting is called. }
    procedure StartAccepting;
    procedure StopAccepting(DoAbort: Boolean = False);
    { Wait for one connection and return its stream. }
    function GetConnection: TSocketStream; virtual;
    property Socket: LongInt read FSocket;
    property QueueSize: Integer read FQueueSize write FQueueSize;
    property MaxConnections: LongInt read FMaxConnections write FMaxConnections;
    { Milliseconds StartAccepting waits for a connection before OnIdle. }
    property AcceptIdleTimeOut: Integer read FAcceptIdleTimeOut write FAcceptIdleTimeOut;
    property OnConnect: TConnectEvent read FOnConnect write FOnConnect;
    property OnConnectQuery: TConnectQuery read FOnConnectQuery write FOnConnectQuery;
    property OnIdle: TNotifyEvent read FOnIdle write FOnIdle;
    property OnCreateClientSocketHandler: TGetClientSocketHandlerEvent
      read FOnCreateClientSocketHandler write FOnCreateClientSocketHandler;
  end;

  TInetServer = class(TSocketServer)
  private
    FHost: string;
    FPort: Word;
  protected
    procedure Bind; override;
    function Accept: LongInt; override;
    function SockToStream(ASocket: LongInt): TSocketStream; override;
  public
    constructor Create(APort: Word); overload;
    constructor Create(const AHost: string; const APort: Word; AHandler: TSocketHandler = nil); overload;
    property Host: string read FHost;
    { The port listened on (the one the system chose, if 0 was asked). }
    property Port: Word read FPort;
  end;

implementation

type
  TBrutoTimeVal = record
    Sec: Int64;
    USec: Int64;
  end;
  TBrutoPollFd = record
    Fd: cint;
    Events: Word;
    REvents: Word;
  end;

const
  BrutoPollIn = 1;
  BrutoPollOut = 4;
  BrutoPollErr = 8;

function bruto_poll(Fds: Pointer; Count: LongWord; Timeout: cint): cint; cdecl; external 'c' name 'poll';
function bruto_ioctl(Fd: cint; Request: culong): cint; cdecl; varargs; external 'c' name 'ioctl';

{ ESocketError }

constructor ESocketError.Create(ACode: TSocketErrorType; const Msg: string);
begin
  inherited Create(Msg);
  FCode := ACode;
end;

{ TSocketHandler }

constructor TSocketHandler.Create;
begin
  inherited Create;
end;

procedure TSocketHandler.SetSocket(const AStream: TSocketStream);
begin
  FSocket := AStream;
end;

procedure TSocketHandler.CheckSocket;
begin
  if FSocket = nil then
    raise ESocketError.Create(seConnectFailed, 'Socket handler has no socket');
end;

function TSocketHandler.Connect: Boolean;
begin
  Result := True;
end;

function TSocketHandler.Accept: Boolean;
begin
  Result := True;
end;

function TSocketHandler.Close: Boolean;
begin
  Result := True;
end;

function TSocketHandler.Shutdown(BiDirectional: Boolean): Boolean;
var
  How: cint;
begin
  CheckSocket;
  if BiDirectional then
    How := SHUT_RDWR
  else
    How := SHUT_WR;
  Result := fpShutdown(FSocket.Handle, How) = 0;
end;

function TSocketHandler.Select(aCheck: TSocketStates; TimeOut: Integer): TSocketStates;
var
  Fd: TBrutoPollFd;
begin
  CheckSocket;
  Result := [];
  Fd.Fd := FSocket.Handle;
  Fd.Events := 0;
  Fd.REvents := 0;
  if sosCanread in aCheck then
    Fd.Events := Fd.Events or BrutoPollIn;
  if sosCanWrite in aCheck then
    Fd.Events := Fd.Events or BrutoPollOut;
  if bruto_poll(@Fd, 1, TimeOut) > 0 then
  begin
    if (Fd.REvents and BrutoPollIn) <> 0 then
      Include(Result, sosCanread);
    if (Fd.REvents and BrutoPollOut) <> 0 then
      Include(Result, sosCanWrite);
    if (Fd.REvents and BrutoPollErr) <> 0 then
      Include(Result, sosException);
  end;
end;

function TSocketHandler.CanRead(TimeOut: Integer): Boolean;
begin
  Result := sosCanread in Select([sosCanread], TimeOut);
end;

function TSocketHandler.Recv(const Buffer; Count: Integer): Integer;
begin
  CheckSocket;
  Result := fpRecv(FSocket.Handle, @Buffer, Count, FSocket.ReadFlags);
  if Result < 0 then
    FLastError := SocketError
  else
    FLastError := 0;
end;

function TSocketHandler.Send(const Buffer; Count: Integer): Integer;
begin
  CheckSocket;
  Result := fpSend(FSocket.Handle, @Buffer, Count, FSocket.WriteFlags);
  if Result < 0 then
    FLastError := SocketError
  else
    FLastError := 0;
end;

function TSocketHandler.BytesAvailable: Integer;
var
  N: cint;
begin
  CheckSocket;
  N := 0;
{$ifdef DARWIN}
  bruto_ioctl(FSocket.Handle, $4004667F, @N);
{$else}
  bruto_ioctl(FSocket.Handle, $541B, @N);
{$endif}
  Result := N;
end;

function TSocketHandler.CreateClientHandler: TSocketHandler;
begin
  Result := TSocketHandlerClass(ClassType).Create;
end;

function TSocketHandler.GetLastErrorDescription: string;
begin
  if FLastError = 0 then
    Result := ''
  else
    Result := SysErrorMessage(FLastError);
end;

{ TSocketStream }

constructor TSocketStream.Create(AHandle: LongInt; AHandler: TSocketHandler);
begin
  inherited Create(AHandle);
  if AHandler = nil then
    AHandler := TSocketHandler.Create;
  FHandler := AHandler;
  FHandler.SetSocket(Self);
end;

destructor TSocketStream.Destroy;
begin
  if not FClosed then
    Close;
  FreeAndNil(FHandler);
  inherited Destroy;
end;

procedure TSocketStream.DoOnClose;
begin
  if Assigned(FOnClose) then
    FOnClose(Self);
end;

procedure TSocketStream.Close;
begin
  if FClosed then
    Exit;
  DoOnClose;
  if FHandler <> nil then
    FHandler.Close;
  if FHandle >= 0 then
    CloseSocket(FHandle);
  FClosed := True;
end;

function TSocketStream.Seek(const Offset: Int64; Origin: TSeekOrigin): Int64;
begin
  Result := 0;
end;

function TSocketStream.Select(aCheck: TSocketStates; TimeOut: Integer): TSocketStates;
begin
  Result := FHandler.Select(aCheck, TimeOut);
end;

function TSocketStream.CanRead(TimeOut: Integer): Boolean;
begin
  Result := FHandler.CanRead(TimeOut);
end;

function TSocketStream.Read(var Buffer; Count: LongInt): LongInt;
begin
  Result := FHandler.Recv(Buffer, Count);
  if Result = 0 then
    FPeerClosed := True;
  if Result < 0 then
    Result := -1;
end;

function TSocketStream.Write(const Buffer; Count: LongInt): LongInt;
begin
  Result := FHandler.Send(Buffer, Count);
end;

function TSocketStream.GetLastError: Integer;
begin
  Result := FHandler.LastError;
end;

procedure TSocketStream.SetIOTimeout(AValue: Integer);
var
  TV: TBrutoTimeVal;
begin
  FIOTimeout := AValue;
  TV.Sec := AValue div 1000;
  TV.USec := (AValue mod 1000) * 1000;
  fpSetSockOpt(FHandle, SOL_SOCKET, SO_RCVTIMEO, @TV, SizeOf(TV));
  fpSetSockOpt(FHandle, SOL_SOCKET, SO_SNDTIMEO, @TV, SizeOf(TV));
end;

function TSocketStream.GetLocalAddress: TSockAddr;
var
  A: TSockAddr;
  L: TSockLen;
begin
  FillChar(A, SizeOf(A), 0);
  L := SizeOf(A);
  fpGetSockName(FHandle, @A, @L);
  Result := A;
end;

function TSocketStream.GetRemoteAddress: TSockAddr;
var
  A: TSockAddr;
  L: TSockLen;
begin
  FillChar(A, SizeOf(A), 0);
  L := SizeOf(A);
  fpGetPeerName(FHandle, @A, @L);
  Result := A;
end;

{ TInetSocket }

constructor TInetSocket.Create(const AHost: string; APort: Word; AHandler: TSocketHandler);
begin
  Create(AHost, APort, 0, AHandler);
end;

constructor TInetSocket.Create(const AHost: string; APort: Word; aConnectTimeout: Integer;
  AHandler: TSocketHandler);
var
  S: LongInt;
begin
  FHost := AHost;
  FPort := APort;
  S := fpSocket(AF_INET, SOCK_STREAM, 0);
  if S < 0 then
    raise ESocketError.Create(seCreationFailed,
      'Creation of socket failed: ' + SysErrorMessage(SocketError));
  inherited Create(S, AHandler);
  ConnectTimeout := aConnectTimeout;
  Connect;
end;

procedure TInetSocket.Connect;
var
  H: THostEntry;
  Addr: TInetSockAddr;
  Ok: Boolean;
begin
  if not ResolveHostByName(FHost, H) then
    raise ESocketError.Create(seHostNotFound, 'Host name resolution for "' + FHost + '" failed.');
  FillChar(Addr, SizeOf(Addr), 0);
{$ifdef DARWIN}
  Addr.sin_len := SizeOf(Addr);
{$endif}
  Addr.sin_family := AF_INET;
  Addr.sin_port := htons(FPort);
  Addr.sin_addr.s_addr := htonl(H.Addr.s_addr);
  if ConnectTimeout > 0 then
    IOTimeout := ConnectTimeout;
  if fpConnect(FHandle, @Addr, SizeOf(Addr)) <> 0 then
    raise ESocketError.Create(seConnectFailed,
      'Connect to ' + FHost + ':' + IntToStr(FPort) + ' failed: ' + SysErrorMessage(SocketError));
  Ok := FHandler.Connect;
  if not Ok then
  begin
    Close;
    raise ESocketError.Create(seConnectFailed,
      'Connect to ' + FHost + ':' + IntToStr(FPort) + ' failed: ' + FHandler.GetLastErrorDescription);
  end;
end;

{ TSocketServer }

constructor TSocketServer.Create(ASocket: LongInt; AHandler: TSocketHandler);
begin
  inherited Create;
  FSocket := ASocket;
  FQueueSize := 5;
  FMaxConnections := -1;
  FAcceptIdleTimeOut := -1;
  FHandler := AHandler;
end;

destructor TSocketServer.Destroy;
begin
  if FSocket >= 0 then
    CloseSocket(FSocket);
  FHandler.Free;
  inherited Destroy;
end;

procedure TSocketServer.Listen;
begin
  if not FBound then
    Bind;
  if fpListen(FSocket, FQueueSize) <> 0 then
    raise ESocketError.Create(seListenFailed,
      'Listening on port #' + IntToStr(FSocket) + ' failed, error: ' + IntToStr(SocketError));
end;

function TSocketServer.GetClientSocketHandler(ASocket: LongInt): TSocketHandler;
begin
  Result := nil;
  if Assigned(FOnCreateClientSocketHandler) then
    FOnCreateClientSocketHandler(Self, Result)
  else if FHandler <> nil then
    Result := FHandler.CreateClientHandler;
end;

function TSocketServer.DoConnectQuery(ASocket: LongInt): Boolean;
begin
  Result := True;
  if Assigned(FOnConnectQuery) then
    FOnConnectQuery(Self, ASocket, Result);
end;

procedure TSocketServer.DoConnect(ASocket: TSocketStream);
begin
  if Assigned(FOnConnect) then
    FOnConnect(Self, ASocket)
  else
    ASocket.Free;
end;

function TSocketServer.GetConnection: TSocketStream;
var
  S: LongInt;
begin
  Result := nil;
  S := Accept;
  if S < 0 then
    raise ESocketError.Create(seAcceptFailed, 'Could not accept a client connection: ' +
      SysErrorMessage(SocketError));
  if not DoConnectQuery(S) then
  begin
    CloseSocket(S);
    Exit;
  end;
  Result := SockToStream(S);
end;

procedure TSocketServer.StartAccepting;
var
  Count: LongInt;
  Stream: TSocketStream;
  Fd: TBrutoPollFd;
begin
  Listen;
  FAccepting := True;
  Count := 0;
  while FAccepting and ((FMaxConnections < 0) or (Count < FMaxConnections)) do
  begin
    if FAcceptIdleTimeOut >= 0 then
    begin
      Fd.Fd := FSocket;
      Fd.Events := BrutoPollIn;
      Fd.REvents := 0;
      if bruto_poll(@Fd, 1, FAcceptIdleTimeOut) <= 0 then
      begin
        if Assigned(FOnIdle) then
          FOnIdle(Self);
        Continue;
      end;
    end;
    Stream := GetConnection;
    if Stream <> nil then
    begin
      Inc(Count);
      DoConnect(Stream);
    end;
  end;
  FAccepting := False;
end;

procedure TSocketServer.StopAccepting(DoAbort: Boolean);
begin
  FAccepting := False;
  if DoAbort then
    fpShutdown(FSocket, SHUT_RDWR);
end;

{ TInetServer }

constructor TInetServer.Create(APort: Word);
begin
  Create('0.0.0.0', APort, nil);
end;

constructor TInetServer.Create(const AHost: string; const APort: Word; AHandler: TSocketHandler);
var
  S: LongInt;
  One: cint;
begin
  FHost := AHost;
  FPort := APort;
  S := fpSocket(AF_INET, SOCK_STREAM, 0);
  if S < 0 then
    raise ESocketError.Create(seCreationFailed,
      'Creation of socket failed: ' + SysErrorMessage(SocketError));
  One := 1;
  fpSetSockOpt(S, SOL_SOCKET, SO_REUSEADDR, @One, SizeOf(One));
  inherited Create(S, AHandler);
end;

procedure TInetServer.Bind;
var
  Addr: TInetSockAddr;
  Len: TSockLen;
begin
  FillChar(Addr, SizeOf(Addr), 0);
{$ifdef DARWIN}
  Addr.sin_len := SizeOf(Addr);
{$endif}
  Addr.sin_family := AF_INET;
  Addr.sin_port := htons(FPort);
  Addr.sin_addr := StrToNetAddr(FHost);
  if fpBind(FSocket, @Addr, SizeOf(Addr)) <> 0 then
    raise ESocketError.Create(seBindFailed, 'Binding of socket failed: ' + SysErrorMessage(SocketError));
  Len := SizeOf(Addr);
  fpGetSockName(FSocket, @Addr, @Len);
  FPort := ntohs(Addr.sin_port);
  FBound := True;
end;

function TInetServer.Accept: LongInt;
begin
  Result := fpAccept(FSocket, nil, nil);
end;

function TInetServer.SockToStream(ASocket: LongInt): TSocketStream;
var
  H: TSocketHandler;
begin
  H := GetClientSocketHandler(ASocket);
  Result := TSocketStream.Create(ASocket, H);
  if not Result.Handler.Accept then
  begin
    Result.Free;
    Result := nil;
  end;
end;

end.
