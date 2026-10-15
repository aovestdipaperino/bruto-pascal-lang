{ Bruto Pascal runtime: FPC's opensslsockets unit. Using it makes
  TOpenSSLSocketHandler the default TLS handler, so TInetSocket,
  TInetServer and TFPHTTPClient (https://) can talk TLS through OpenSSL. }
unit opensslsockets;

interface

uses SysUtils, Classes, sslbase, ssockets, sslsockets, openssl;

type
  TOpenSSLSocketHandler = class(TSSLSocketHandler)
  private
    FSSL: PSSL;
    FCTX: PSSL_CTX;
    FSSLLastError: Integer;
    FSSLLastErrorString: string;
    function InitContext(Server: Boolean): Boolean;
    procedure DoneContext;
    function SetFailure(Code: Integer): Boolean;
    function PeerHost: string;
  protected
    function GetLastSSLErrorString: string; override;
    function GetLastSSLErrorCode: Integer; override;
  public
    constructor Create; override;
    destructor Destroy; override;
    function Connect: Boolean; override;
    function Accept: Boolean; override;
    function Close: Boolean; override;
    function Shutdown(BiDirectional: Boolean): Boolean; override;
    function CanRead(TimeOut: Integer): Boolean; override;
    function Recv(const Buffer; Count: Integer): Integer; override;
    function Send(const Buffer; Count: Integer): Integer; override;
    function BytesAvailable: Integer; override;
    property SSL: PSSL read FSSL;
    property CTX: PSSL_CTX read FCTX;
  end;

implementation

constructor TOpenSSLSocketHandler.Create;
begin
  inherited Create;
  if not InitSSLInterface then
    raise ESSLSocketError.Create(seCreationFailed,
      'Could not load the OpenSSL libraries (libssl, libcrypto)');
end;

destructor TOpenSSLSocketHandler.Destroy;
begin
  DoneContext;
  inherited Destroy;
end;

function TOpenSSLSocketHandler.SetFailure(Code: Integer): Boolean;
var
  E: cInt;
begin
  FSSLLastError := Code;
  E := ErrGetError;
  if E <> 0 then
    FSSLLastErrorString := ErrErrorString(E)
  else
    FSSLLastErrorString := 'SSL error code: ' + IntToStr(Code);
  Result := False;
end;

function TOpenSSLSocketHandler.GetLastSSLErrorString: string;
begin
  Result := FSSLLastErrorString;
end;

function TOpenSSLSocketHandler.GetLastSSLErrorCode: Integer;
begin
  Result := FSSLLastError;
end;

function TOpenSSLSocketHandler.PeerHost: string;
begin
  Result := CertificateData.HostName;
  if (Result = '') and (Socket is TInetSocket) then
    Result := TInetSocket(Socket).Host;
end;

function TOpenSSLSocketHandler.InitContext(Server: Boolean): Boolean;
var
  Ciphers: AnsiString;
begin
  Result := False;
  ErrClearError;
  if Server then
    FCTX := SslCtxNew(SslServerMethod)
  else
    FCTX := SslCtxNew(SslTLSMethod);
  if FCTX = nil then
    Exit(SetFailure(SSL_ERROR_SSL));
  if VerifyPeerCert then
  begin
    SslCtxSetVerify(FCTX, SSL_VERIFY_PEER, nil);
    if not CertificateData.TrustedCertificate.Empty and
      (CertificateData.TrustedCertificate.FileName <> '') then
      SslCtxLoadVerifyLocations(FCTX, CertificateData.TrustedCertificate.FileName, '')
    else
      SslCtxSetDefaultVerifyPaths(FCTX);
  end
  else
    SslCtxSetVerify(FCTX, SSL_VERIFY_NONE, nil);
  if (CertificateData.CipherList <> '') and (CertificateData.CipherList <> 'DEFAULT') then
  begin
    Ciphers := CertificateData.CipherList;
    SslCtxSetCipherList(FCTX, Ciphers);
  end;
  if CertificateData.Certificate.FileName <> '' then
    if SslCtxUseCertificateChainFile(FCTX, CertificateData.Certificate.FileName) <= 0 then
      Exit(SetFailure(SSL_ERROR_SSL));
  if CertificateData.PrivateKey.FileName <> '' then
    if SslCtxUsePrivateKeyFile(FCTX, CertificateData.PrivateKey.FileName, SSL_FILETYPE_PEM) <= 0 then
      Exit(SetFailure(SSL_ERROR_SSL));
  FSSL := SslNew(FCTX);
  if FSSL = nil then
    Exit(SetFailure(SSL_ERROR_SSL));
  if SslSetFd(FSSL, Socket.Handle) <= 0 then
    Exit(SetFailure(SSL_ERROR_SSL));
  Result := True;
end;

procedure TOpenSSLSocketHandler.DoneContext;
begin
  if FSSL <> nil then
    SslFree(FSSL);
  FSSL := nil;
  if FCTX <> nil then
    SslCtxFree(FCTX);
  FCTX := nil;
  SetSSLActive(False);
end;

function TOpenSSLSocketHandler.Connect: Boolean;
var
  Host: string;
  R: cInt;
begin
  Result := inherited Connect;
  if not Result then
    Exit;
  if not InitContext(False) then
    Exit(False);
  Host := PeerHost;
  if SendHostAsSNI and (Host <> '') then
    SslCtrl(FSSL, SSL_CTRL_SET_TLSEXT_HOSTNAME, TLSEXT_NAMETYPE_host_name, PChar(Host));
  if VerifyPeerCert and (Host <> '') then
    SslSet1Host(FSSL, Host);
  R := SslConnect(FSSL);
  if R <= 0 then
    Exit(SetFailure(SslGetError(FSSL, R)));
  if VerifyPeerCert and (SSLGetVerifyResult(FSSL) <> X509_V_OK) then
    Exit(SetFailure(SSL_ERROR_SSL));
  Result := DoVerifyCert;
  SetSSLActive(Result);
end;

function TOpenSSLSocketHandler.Accept: Boolean;
var
  R: cInt;
begin
  Result := inherited Accept;
  if not Result then
    Exit;
  if not InitContext(True) then
    Exit(False);
  R := SslAccept(FSSL);
  if R <= 0 then
    Exit(SetFailure(SslGetError(FSSL, R)));
  SetSSLActive(True);
  Result := True;
end;

function TOpenSSLSocketHandler.Shutdown(BiDirectional: Boolean): Boolean;
begin
  if SSLActive and (FSSL <> nil) then
    SslShutdown(FSSL);
  Result := inherited Shutdown(BiDirectional);
end;

function TOpenSSLSocketHandler.Close: Boolean;
begin
  if SSLActive and (FSSL <> nil) then
    SslShutdown(FSSL);
  DoneContext;
  Result := True;
end;

function TOpenSSLSocketHandler.CanRead(TimeOut: Integer): Boolean;
begin
  if (FSSL <> nil) and (SslPending(FSSL) > 0) then
    Result := True
  else
    Result := inherited CanRead(TimeOut);
end;

function TOpenSSLSocketHandler.Recv(const Buffer; Count: Integer): Integer;
var
  E: cInt;
begin
  if not SSLActive then
    Exit(inherited Recv(Buffer, Count));
  repeat
    Result := SslRead(FSSL, @Buffer, Count);
    if Result > 0 then
      Exit;
    E := SslGetError(FSSL, Result);
  until (E <> SSL_ERROR_WANT_READ) and (E <> SSL_ERROR_WANT_WRITE);
  if E = SSL_ERROR_ZERO_RETURN then
    Exit(0);
  FLastError := E;
  SetFailure(E);
  { A connection closed without a TLS close_notify ends the data too. }
  if E = SSL_ERROR_SYSCALL then
    Result := 0
  else
    Result := -1;
end;

function TOpenSSLSocketHandler.Send(const Buffer; Count: Integer): Integer;
var
  E: cInt;
begin
  if not SSLActive then
    Exit(inherited Send(Buffer, Count));
  repeat
    Result := SslWrite(FSSL, @Buffer, Count);
    if Result > 0 then
      Exit;
    E := SslGetError(FSSL, Result);
  until (E <> SSL_ERROR_WANT_READ) and (E <> SSL_ERROR_WANT_WRITE);
  FLastError := E;
  SetFailure(E);
  Result := -1;
end;

function TOpenSSLSocketHandler.BytesAvailable: Integer;
begin
  if FSSL <> nil then
    Result := SslPending(FSSL)
  else
    Result := inherited BytesAvailable;
end;

initialization
  TSSLSocketHandler.SetDefaultHandlerClass(TOpenSSLSocketHandler);
end.
