{ Bruto Pascal runtime: the part of FPC's openssl unit TLS needs, with
  the same names. libssl and libcrypto (OpenSSL 1.1 or 3) are loaded
  when InitSSLInterface is first called: on macOS from Homebrew, on
  Linux the system's, on Windows libssl-3-x64.dll. Without them,
  InitSSLInterface returns False. }
unit OpenSSL;

interface

uses UnixType;

type
  SslPtr = Pointer;
  PSSL = SslPtr;
  PSSL_CTX = SslPtr;
  PSSL_METHOD = SslPtr;
  PX509 = SslPtr;

const
  SSL_ERROR_NONE = 0;
  SSL_ERROR_SSL = 1;
  SSL_ERROR_WANT_READ = 2;
  SSL_ERROR_WANT_WRITE = 3;
  SSL_ERROR_WANT_X509_LOOKUP = 4;
  SSL_ERROR_SYSCALL = 5;
  SSL_ERROR_ZERO_RETURN = 6;
  SSL_ERROR_WANT_CONNECT = 7;
  SSL_ERROR_WANT_ACCEPT = 8;

  SSL_VERIFY_NONE = 0;
  SSL_VERIFY_PEER = 1;
  SSL_VERIFY_FAIL_IF_NO_PEER_CERT = 2;

  SSL_FILETYPE_PEM = 1;
  SSL_FILETYPE_ASN1 = 2;

  SSL_CTRL_SET_TLSEXT_HOSTNAME = 55;
  TLSEXT_NAMETYPE_host_name = 0;
  X509_V_OK = 0;

var
  { The libraries InitSSLInterface found (empty until then). }
  DLLSSLName: AnsiString;
  DLLUtilName: AnsiString;

{ Load libssl and libcrypto; whether they could be. }
function InitSSLInterface: Boolean;
function IsSSLloaded: Boolean;

function SslLibraryInit: cInt;
procedure SslLoadErrorStrings;
function SslMethodV23: PSSL_METHOD;
function SslTLSMethod: PSSL_METHOD;
function SslMethodTLSV1_2: PSSL_METHOD;
function SslServerMethod: PSSL_METHOD;
function SslCtxNew(meth: PSSL_METHOD): PSSL_CTX;
procedure SslCtxFree(arg0: PSSL_CTX);
function SslCtxSetDefaultVerifyPaths(ctx: PSSL_CTX): cInt;
function SslCtxLoadVerifyLocations(ctx: PSSL_CTX; const CAfile: AnsiString; const CApath: AnsiString): cInt;
procedure SslCtxSetVerify(ctx: PSSL_CTX; mode: cInt; arg2: Pointer);
function SslCtxUseCertificateFile(ctx: PSSL_CTX; const _file: AnsiString; _type: cInt): cInt;
function SslCtxUseCertificateChainFile(ctx: PSSL_CTX; const _file: AnsiString): cInt;
function SslCtxUsePrivateKeyFile(ctx: PSSL_CTX; const _file: AnsiString; _type: cInt): cInt;
function SslCtxCheckPrivateKeyFile(ctx: PSSL_CTX): cInt;
function SslCtxSetCipherList(arg0: PSSL_CTX; var str: AnsiString): cInt;
function SslNew(ctx: PSSL_CTX): PSSL;
procedure SslFree(ssl: PSSL);
function SslSetFd(s: PSSL; fd: cInt): cInt;
function SslCtrl(ssl: PSSL; cmd: cInt; larg: clong; parg: Pointer): clong;
function SslSet1Host(ssl: PSSL; hostname: string): cInt;
function SslConnect(ssl: PSSL): cInt;
function SslAccept(ssl: PSSL): cInt;
function SslShutdown(ssl: PSSL): cInt;
function SslRead(ssl: PSSL; buf: SslPtr; num: cInt): cInt;
function SslWrite(ssl: PSSL; buf: SslPtr; num: cInt): cInt;
function SslPending(ssl: PSSL): cInt;
function SslGetError(s: PSSL; ret_code: cInt): cInt;
function SSLGetVerifyResult(ssl: PSSL): clong;
function SslGetVersion(ssl: PSSL): AnsiString;
function ErrGetError: cInt;
procedure ErrClearError;
{ The text of OpenSSL error code e. }
function ErrErrorString(e: cInt): AnsiString;
{ OpenSSL's version, as `OpenSSL 3.4.1 11 Feb 2025`. }
function OpenSSLGetVersion: AnsiString;

implementation

type
  TSslFn0 = function: Pointer;
  TSslFnP = function(P: Pointer): Pointer;
  TSslFnPI = function(P: Pointer): cInt;
  TSslProcP = procedure(P: Pointer);
  TSslFnPPI = function(P: Pointer; Q: Pointer): cInt;
  TSslFnPPPI = function(P: Pointer; Q: Pointer; R: Pointer): cInt;
  TSslFnPPII = function(P: Pointer; Q: Pointer; T: cInt): cInt;
  TSslProcPIP = procedure(P: Pointer; Mode: cInt; Cb: Pointer);
  TSslFnPII = function(P: Pointer; I: cInt): cInt;
  TSslFnPIIP = function(P: Pointer; Cmd: cInt; L: clong; Q: Pointer): clong;
  TSslFnPPII2 = function(P: Pointer; Buf: Pointer; N: cInt): cInt;
  TSslFnPL = function(P: Pointer): clong;
  TSslFnL = function: culong;
  TSslProc = procedure;
  TSslProcLPS = procedure(E: culong; Buf: PChar; Len: PtrUInt);
  TSslFnIP = function(T: cInt): PChar;
  TSslFnInit = function(Opts: cuint64; Settings: Pointer): cInt;

var
  SslLoaded: Boolean;
  SslTried: Boolean;
  _OPENSSL_init_ssl: TSslFnInit;
  _TLS_client_method: TSslFn0;
  _TLS_server_method: TSslFn0;
  _TLS_method: TSslFn0;
  _SSL_CTX_new: TSslFnP;
  _SSL_CTX_free: TSslProcP;
  _SSL_CTX_set_default_verify_paths: TSslFnPI;
  _SSL_CTX_load_verify_locations: TSslFnPPPI;
  _SSL_CTX_set_verify: TSslProcPIP;
  _SSL_CTX_use_certificate_file: TSslFnPPII;
  _SSL_CTX_use_certificate_chain_file: TSslFnPPI;
  _SSL_CTX_use_PrivateKey_file: TSslFnPPII;
  _SSL_CTX_check_private_key: TSslFnPI;
  _SSL_CTX_set_cipher_list: TSslFnPPI;
  _SSL_new: TSslFnP;
  _SSL_free: TSslProcP;
  _SSL_set_fd: TSslFnPII;
  _SSL_ctrl: TSslFnPIIP;
  _SSL_set1_host: TSslFnPPI;
  _SSL_connect: TSslFnPI;
  _SSL_accept: TSslFnPI;
  _SSL_shutdown: TSslFnPI;
  _SSL_read: TSslFnPPII2;
  _SSL_write: TSslFnPPII2;
  _SSL_pending: TSslFnPI;
  _SSL_get_error: TSslFnPII;
  _SSL_get_verify_result: TSslFnPL;
  _SSL_get_version: TSslFnP;
  _ERR_get_error: TSslFnL;
  _ERR_clear_error: TSslProc;
  _ERR_error_string_n: TSslProcLPS;
  _OpenSSL_version: TSslFnIP;

{$ifdef WINDOWS}
function bruto_LoadLibraryA(Name: PChar): Pointer; cdecl; external 'kernel32' name 'LoadLibraryA';
function bruto_GetProcAddress(Lib: Pointer; Name: PChar): Pointer; cdecl; external 'kernel32' name 'GetProcAddress';

function OpenLibrary(const Name: AnsiString): Pointer;
begin
  OpenLibrary := bruto_LoadLibraryA(PChar(Name));
end;

function LibrarySymbol(Lib: Pointer; const Name: AnsiString): Pointer;
begin
  LibrarySymbol := bruto_GetProcAddress(Lib, PChar(Name));
end;
{$else}
function bruto_dlopen(Name: PChar; Flags: cInt): Pointer; cdecl; external 'c' name 'dlopen';
function bruto_dlsym(Lib: Pointer; Name: PChar): Pointer; cdecl; external 'c' name 'dlsym';

function OpenLibrary(const Name: AnsiString): Pointer;
begin
  { RTLD_NOW }
  OpenLibrary := bruto_dlopen(PChar(Name), 2);
end;

function LibrarySymbol(Lib: Pointer; const Name: AnsiString): Pointer;
begin
  LibrarySymbol := bruto_dlsym(Lib, PChar(Name));
end;
{$endif}

{ The libssl/libcrypto pairs to try, in order. (macOS's own
  /usr/lib/libssl.dylib stops the program when loaded, so only explicit
  Homebrew paths are tried there.) }
const
{$ifdef DARWIN}
  SslCandidateCount = 6;
  SslCandidates: array[1..6] of string = (
    '/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib',
    '/opt/homebrew/opt/openssl/lib/libssl.dylib',
    '/usr/local/opt/openssl@3/lib/libssl.3.dylib',
    '/usr/local/opt/openssl/lib/libssl.dylib',
    '/opt/local/lib/libssl.3.dylib',
    '/opt/homebrew/opt/openssl@1.1/lib/libssl.1.1.dylib');
  CryptoCandidates: array[1..6] of string = (
    '/opt/homebrew/opt/openssl@3/lib/libcrypto.3.dylib',
    '/opt/homebrew/opt/openssl/lib/libcrypto.dylib',
    '/usr/local/opt/openssl@3/lib/libcrypto.3.dylib',
    '/usr/local/opt/openssl/lib/libcrypto.dylib',
    '/opt/local/lib/libcrypto.3.dylib',
    '/opt/homebrew/opt/openssl@1.1/lib/libcrypto.1.1.dylib');
{$else}
{$ifdef WINDOWS}
  SslCandidateCount = 2;
  SslCandidates: array[1..2] of string = ('libssl-3-x64.dll', 'libssl-1_1-x64.dll');
  CryptoCandidates: array[1..2] of string = ('libcrypto-3-x64.dll', 'libcrypto-1_1-x64.dll');
{$else}
  SslCandidateCount = 3;
  SslCandidates: array[1..3] of string = ('libssl.so.3', 'libssl.so.1.1', 'libssl.so');
  CryptoCandidates: array[1..3] of string = ('libcrypto.so.3', 'libcrypto.so.1.1', 'libcrypto.so');
{$endif}
{$endif}

function InitSSLInterface: Boolean;
var
  I: Integer;
  SslLib, CryptoLib: Pointer;
begin
  if SslTried then
    Exit(SslLoaded);
  SslTried := True;
  SslLib := nil;
  CryptoLib := nil;
  for I := 1 to SslCandidateCount do
    if SslLib = nil then
    begin
      CryptoLib := OpenLibrary(CryptoCandidates[I]);
      if CryptoLib <> nil then
      begin
        SslLib := OpenLibrary(SslCandidates[I]);
        if SslLib <> nil then
        begin
          DLLSSLName := SslCandidates[I];
          DLLUtilName := CryptoCandidates[I];
        end;
      end;
    end;
  if SslLib = nil then
    Exit(False);
  _OPENSSL_init_ssl := TSslFnInit(LibrarySymbol(SslLib, 'OPENSSL_init_ssl'));
  _TLS_client_method := TSslFn0(LibrarySymbol(SslLib, 'TLS_client_method'));
  _TLS_server_method := TSslFn0(LibrarySymbol(SslLib, 'TLS_server_method'));
  _TLS_method := TSslFn0(LibrarySymbol(SslLib, 'TLS_method'));
  _SSL_CTX_new := TSslFnP(LibrarySymbol(SslLib, 'SSL_CTX_new'));
  _SSL_CTX_free := TSslProcP(LibrarySymbol(SslLib, 'SSL_CTX_free'));
  _SSL_CTX_set_default_verify_paths := TSslFnPI(LibrarySymbol(SslLib, 'SSL_CTX_set_default_verify_paths'));
  _SSL_CTX_load_verify_locations := TSslFnPPPI(LibrarySymbol(SslLib, 'SSL_CTX_load_verify_locations'));
  _SSL_CTX_set_verify := TSslProcPIP(LibrarySymbol(SslLib, 'SSL_CTX_set_verify'));
  _SSL_CTX_use_certificate_file := TSslFnPPII(LibrarySymbol(SslLib, 'SSL_CTX_use_certificate_file'));
  _SSL_CTX_use_certificate_chain_file := TSslFnPPI(LibrarySymbol(SslLib, 'SSL_CTX_use_certificate_chain_file'));
  _SSL_CTX_use_PrivateKey_file := TSslFnPPII(LibrarySymbol(SslLib, 'SSL_CTX_use_PrivateKey_file'));
  _SSL_CTX_check_private_key := TSslFnPI(LibrarySymbol(SslLib, 'SSL_CTX_check_private_key'));
  _SSL_CTX_set_cipher_list := TSslFnPPI(LibrarySymbol(SslLib, 'SSL_CTX_set_cipher_list'));
  _SSL_new := TSslFnP(LibrarySymbol(SslLib, 'SSL_new'));
  _SSL_free := TSslProcP(LibrarySymbol(SslLib, 'SSL_free'));
  _SSL_set_fd := TSslFnPII(LibrarySymbol(SslLib, 'SSL_set_fd'));
  _SSL_ctrl := TSslFnPIIP(LibrarySymbol(SslLib, 'SSL_ctrl'));
  _SSL_set1_host := TSslFnPPI(LibrarySymbol(SslLib, 'SSL_set1_host'));
  _SSL_connect := TSslFnPI(LibrarySymbol(SslLib, 'SSL_connect'));
  _SSL_accept := TSslFnPI(LibrarySymbol(SslLib, 'SSL_accept'));
  _SSL_shutdown := TSslFnPI(LibrarySymbol(SslLib, 'SSL_shutdown'));
  _SSL_read := TSslFnPPII2(LibrarySymbol(SslLib, 'SSL_read'));
  _SSL_write := TSslFnPPII2(LibrarySymbol(SslLib, 'SSL_write'));
  _SSL_pending := TSslFnPI(LibrarySymbol(SslLib, 'SSL_pending'));
  _SSL_get_error := TSslFnPII(LibrarySymbol(SslLib, 'SSL_get_error'));
  _SSL_get_verify_result := TSslFnPL(LibrarySymbol(SslLib, 'SSL_get_verify_result'));
  _SSL_get_version := TSslFnP(LibrarySymbol(SslLib, 'SSL_get_version'));
  _ERR_get_error := TSslFnL(LibrarySymbol(CryptoLib, 'ERR_get_error'));
  _ERR_clear_error := TSslProc(LibrarySymbol(CryptoLib, 'ERR_clear_error'));
  _ERR_error_string_n := TSslProcLPS(LibrarySymbol(CryptoLib, 'ERR_error_string_n'));
  _OpenSSL_version := TSslFnIP(LibrarySymbol(CryptoLib, 'OpenSSL_version'));
  SslLoaded := (_SSL_CTX_new <> nil) and (_SSL_new <> nil) and (_TLS_method <> nil);
  if SslLoaded and (_OPENSSL_init_ssl <> nil) then
    _OPENSSL_init_ssl(0, nil);
  InitSSLInterface := SslLoaded;
end;

function IsSSLloaded: Boolean;
begin
  IsSSLloaded := SslLoaded;
end;

function SslLibraryInit: cInt;
begin
  if InitSSLInterface then
    SslLibraryInit := 1
  else
    SslLibraryInit := 0;
end;

procedure SslLoadErrorStrings;
begin
end;

function SslMethodV23: PSSL_METHOD;
begin
  SslMethodV23 := _TLS_method();
end;

function SslTLSMethod: PSSL_METHOD;
begin
  SslTLSMethod := _TLS_client_method();
end;

function SslMethodTLSV1_2: PSSL_METHOD;
begin
  SslMethodTLSV1_2 := _TLS_method();
end;

function SslServerMethod: PSSL_METHOD;
begin
  SslServerMethod := _TLS_server_method();
end;

function SslCtxNew(meth: PSSL_METHOD): PSSL_CTX;
begin
  SslCtxNew := _SSL_CTX_new(meth);
end;

procedure SslCtxFree(arg0: PSSL_CTX);
begin
  if arg0 <> nil then
    _SSL_CTX_free(arg0);
end;

function SslCtxSetDefaultVerifyPaths(ctx: PSSL_CTX): cInt;
begin
  SslCtxSetDefaultVerifyPaths := _SSL_CTX_set_default_verify_paths(ctx);
end;

function SslCtxLoadVerifyLocations(ctx: PSSL_CTX; const CAfile: AnsiString; const CApath: AnsiString): cInt;
var
  F, P: PChar;
begin
  F := nil;
  P := nil;
  if CAfile <> '' then
    F := PChar(CAfile);
  if CApath <> '' then
    P := PChar(CApath);
  SslCtxLoadVerifyLocations := _SSL_CTX_load_verify_locations(ctx, F, P);
end;

procedure SslCtxSetVerify(ctx: PSSL_CTX; mode: cInt; arg2: Pointer);
begin
  _SSL_CTX_set_verify(ctx, mode, arg2);
end;

function SslCtxUseCertificateFile(ctx: PSSL_CTX; const _file: AnsiString; _type: cInt): cInt;
begin
  SslCtxUseCertificateFile := _SSL_CTX_use_certificate_file(ctx, PChar(_file), _type);
end;

function SslCtxUseCertificateChainFile(ctx: PSSL_CTX; const _file: AnsiString): cInt;
begin
  SslCtxUseCertificateChainFile := _SSL_CTX_use_certificate_chain_file(ctx, PChar(_file));
end;

function SslCtxUsePrivateKeyFile(ctx: PSSL_CTX; const _file: AnsiString; _type: cInt): cInt;
begin
  SslCtxUsePrivateKeyFile := _SSL_CTX_use_PrivateKey_file(ctx, PChar(_file), _type);
end;

function SslCtxCheckPrivateKeyFile(ctx: PSSL_CTX): cInt;
begin
  SslCtxCheckPrivateKeyFile := _SSL_CTX_check_private_key(ctx);
end;

function SslCtxSetCipherList(arg0: PSSL_CTX; var str: AnsiString): cInt;
begin
  SslCtxSetCipherList := _SSL_CTX_set_cipher_list(arg0, PChar(str));
end;

function SslNew(ctx: PSSL_CTX): PSSL;
begin
  SslNew := _SSL_new(ctx);
end;

procedure SslFree(ssl: PSSL);
begin
  if ssl <> nil then
    _SSL_free(ssl);
end;

function SslSetFd(s: PSSL; fd: cInt): cInt;
begin
  SslSetFd := _SSL_set_fd(s, fd);
end;

function SslCtrl(ssl: PSSL; cmd: cInt; larg: clong; parg: Pointer): clong;
begin
  SslCtrl := _SSL_ctrl(ssl, cmd, larg, parg);
end;

function SslSet1Host(ssl: PSSL; hostname: string): cInt;
begin
  if _SSL_set1_host = nil then
    Exit(0);
  SslSet1Host := _SSL_set1_host(ssl, PChar(hostname));
end;

function SslConnect(ssl: PSSL): cInt;
begin
  SslConnect := _SSL_connect(ssl);
end;

function SslAccept(ssl: PSSL): cInt;
begin
  SslAccept := _SSL_accept(ssl);
end;

function SslShutdown(ssl: PSSL): cInt;
begin
  SslShutdown := _SSL_shutdown(ssl);
end;

function SslRead(ssl: PSSL; buf: SslPtr; num: cInt): cInt;
begin
  SslRead := _SSL_read(ssl, buf, num);
end;

function SslWrite(ssl: PSSL; buf: SslPtr; num: cInt): cInt;
begin
  SslWrite := _SSL_write(ssl, buf, num);
end;

function SslPending(ssl: PSSL): cInt;
begin
  SslPending := _SSL_pending(ssl);
end;

function SslGetError(s: PSSL; ret_code: cInt): cInt;
begin
  SslGetError := _SSL_get_error(s, ret_code);
end;

function SSLGetVerifyResult(ssl: PSSL): clong;
begin
  SSLGetVerifyResult := _SSL_get_verify_result(ssl);
end;

function SslGetVersion(ssl: PSSL): AnsiString;
var
  S: AnsiString;
begin
  S := PChar(_SSL_get_version(ssl));
  SslGetVersion := S;
end;

function ErrGetError: cInt;
begin
  if _ERR_get_error = nil then
    Exit(0);
  ErrGetError := cInt(_ERR_get_error());
end;

procedure ErrClearError;
begin
  if _ERR_clear_error <> nil then
    _ERR_clear_error();
end;

function ErrErrorString(e: cInt): AnsiString;
var
  Buf: array[0..255] of Char;
  S: AnsiString;
begin
  S := '';
  if _ERR_error_string_n <> nil then
  begin
    FillChar(Buf, SizeOf(Buf), 0);
    _ERR_error_string_n(culong(e) and $FFFFFFFF, @Buf, SizeOf(Buf));
    S := PChar(@Buf);
  end;
  ErrErrorString := S;
end;

function OpenSSLGetVersion: AnsiString;
var
  S: AnsiString;
begin
  S := '';
  if _OpenSSL_version <> nil then
    S := _OpenSSL_version(0);
  OpenSSLGetVersion := S;
end;

end.
