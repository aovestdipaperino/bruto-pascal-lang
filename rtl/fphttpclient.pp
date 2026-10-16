{ Bruto Pascal runtime: the core of FPC's fphttpclient unit. HTTP/1.1
  requests over ssockets; https:// URLs use the default TLS handler, so
  a program fetching them also uses opensslsockets. Redirects are
  followed when AllowRedirect is set; chunked and Content-Length bodies
  are read. As in FPC, Get raises EHTTPClient for a status other than
  200, and certificates are checked only with VerifySSLCertificate. }
unit fphttpclient;

{$mode objfpc}{$H+}

interface

uses SysUtils, Classes, ssockets, sslsockets;

type
  EHTTPClient = class(Exception)
  private
    FStatusCode: Integer;
  public
    property StatusCode: Integer read FStatusCode write FStatusCode;
  end;

  TFPCustomHTTPClient = class(TComponent)
  private
    FRequestHeaders: TStrings;
    FResponseHeaders: TStrings;
    FResponseStatusCode: Integer;
    FResponseStatusText: string;
    FServerHTTPVersion: string;
    FHTTPVersion: string;
    FAllowRedirect: Boolean;
    FMaxRedirects: Byte;
    FRequestBody: TStream;
    FIOTimeout: Integer;
    FConnectTimeout: Integer;
    FKeepConnection: Boolean;
    FUserName: string;
    FPassword: string;
    FVerifySSLCertificate: Boolean;
    FSocket: TInetSocket;
    FBuffer: string;
    procedure SetRequestHeaders(AValue: TStrings);
    function ReadLine: string;
    function ReadToBuffer: Boolean;
    procedure ReadBody(Stream: TStream);
    procedure DoRequest(const AMethod, AURL: string; Stream: TStream; AllowedCode: Integer);
  public
    constructor Create(AOwner: TComponent); override;
    destructor Destroy; override;
    { Send AMethod to AURL, with RequestBody as the body if set, and put
      the response's body into Stream. Raises EHTTPClient when the
      status isn't AllowedCode (any status if AllowedCode is 0). }
    procedure HTTPMethod(const AMethod, AURL: string; Stream: TStream; AllowedCode: Integer = 0);
    function Get(const AURL: string): string; overload;
    procedure Get(const AURL: string; Stream: TStream); overload;
    procedure Get(const AURL: string; const LocalFileName: string); overload;
    class function SimpleGet(const AURL: string): string; overload;
    class procedure SimpleGet(const AURL: string; Stream: TStream); overload;
    function Post(const URL: string): string; overload;
    procedure Post(const URL: string; const Response: TStream); overload;
    function FormPost(const URL, FormData: string): string;
    class function SimpleFormPost(const URL, FormData: string): string;
    function Put(const URL: string): string;
    function Delete(const URL: string): string;
    procedure Head(const AURL: string; Headers: TStrings);
    procedure AddHeader(const AHeader, AValue: string);
    { The index of header AHeader in RequestHeaders, and its value. }
    function IndexOfHeader(const AHeader: string): Integer; overload;
    function GetHeader(const AHeader: string): string; overload;
    class function IndexOfHeader(HTTPHeaders: TStrings; const AHeader: string): Integer; overload;
    class function GetHeader(HTTPHeaders: TStrings; const AHeader: string): string; overload;
    property RequestHeaders: TStrings read FRequestHeaders write SetRequestHeaders;
    property ResponseHeaders: TStrings read FResponseHeaders;
    property ResponseStatusCode: Integer read FResponseStatusCode;
    property ResponseStatusText: string read FResponseStatusText;
    property ServerHTTPVersion: string read FServerHTTPVersion;
    property HTTPversion: string read FHTTPVersion write FHTTPVersion;
    property AllowRedirect: Boolean read FAllowRedirect write FAllowRedirect;
    property MaxRedirects: Byte read FMaxRedirects write FMaxRedirects;
    property RequestBody: TStream read FRequestBody write FRequestBody;
    property IOTimeout: Integer read FIOTimeout write FIOTimeout;
    property ConnectTimeout: Integer read FConnectTimeout write FConnectTimeout;
    property KeepConnection: Boolean read FKeepConnection write FKeepConnection;
    property UserName: string read FUserName write FUserName;
    property Password: string read FPassword write FPassword;
    property VerifySSLCertificate: Boolean read FVerifySSLCertificate write FVerifySSLCertificate;
  end;

  TFPHTTPClient = class(TFPCustomHTTPClient);

implementation

const
  Base64Chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function EncodeBase64(const S: string): string;
var
  I, N, B: Integer;
  R: string;
begin
  R := '';
  I := 1;
  N := Length(S);
  while I <= N do
  begin
    B := Ord(S[I]) shl 16;
    if I + 1 <= N then
      B := B or (Ord(S[I + 1]) shl 8);
    if I + 2 <= N then
      B := B or Ord(S[I + 2]);
    R := R + Base64Chars[((B shr 18) and 63) + 1] + Base64Chars[((B shr 12) and 63) + 1];
    if I + 1 <= N then
      R := R + Base64Chars[((B shr 6) and 63) + 1]
    else
      R := R + '=';
    if I + 2 <= N then
      R := R + Base64Chars[(B and 63) + 1]
    else
      R := R + '=';
    I := I + 3;
  end;
  EncodeBase64 := R;
end;

{ Split URL into its parts. }
procedure SplitURL(const URL: string; var Protocol, Host, Path: string; var Port: Word);
var
  Rest: string;
  P, Q: Integer;
begin
  Rest := URL;
  P := Pos('://', Rest);
  if P > 0 then
  begin
    Protocol := LowerCase(Copy(Rest, 1, P - 1));
    Rest := Copy(Rest, P + 3, Length(Rest));
  end
  else
    Protocol := 'http';
  P := Pos('/', Rest);
  Q := Pos('?', Rest);
  if (Q > 0) and ((P = 0) or (Q < P)) then
    P := Q;
  if P > 0 then
  begin
    Host := Copy(Rest, 1, P - 1);
    Path := Copy(Rest, P, Length(Rest));
    if Path[1] = '?' then
      Path := '/' + Path;
  end
  else
  begin
    Host := Rest;
    Path := '/';
  end;
  P := Pos('@', Host);
  if P > 0 then
    Host := Copy(Host, P + 1, Length(Host));
  if Protocol = 'https' then
    Port := 443
  else
    Port := 80;
  P := Pos(':', Host);
  if P > 0 then
  begin
    Port := StrToInt(Copy(Host, P + 1, Length(Host)));
    Host := Copy(Host, 1, P - 1);
  end;
  P := Pos('#', Path);
  if P > 0 then
    Path := Copy(Path, 1, P - 1);
end;

{ TFPCustomHTTPClient }

constructor TFPCustomHTTPClient.Create(AOwner: TComponent);
begin
  inherited Create(AOwner);
  FRequestHeaders := TStringList.Create;
  FResponseHeaders := TStringList.Create;
  FHTTPVersion := '1.1';
  FMaxRedirects := 16;
end;

destructor TFPCustomHTTPClient.Destroy;
begin
  FreeAndNil(FSocket);
  FRequestHeaders.Free;
  FResponseHeaders.Free;
  inherited Destroy;
end;

procedure TFPCustomHTTPClient.SetRequestHeaders(AValue: TStrings);
begin
  FRequestHeaders.Assign(AValue);
end;

class function TFPCustomHTTPClient.IndexOfHeader(HTTPHeaders: TStrings; const AHeader: string): Integer;
var
  H: string;
begin
  H := LowerCase(AHeader) + ':';
  Result := HTTPHeaders.Count - 1;
  while (Result >= 0) and (Pos(H, LowerCase(HTTPHeaders[Result])) <> 1) do
    Dec(Result);
end;

class function TFPCustomHTTPClient.GetHeader(HTTPHeaders: TStrings; const AHeader: string): string;
var
  I: Integer;
begin
  I := IndexOfHeader(HTTPHeaders, AHeader);
  if I < 0 then
    Result := ''
  else
  begin
    Result := HTTPHeaders[I];
    Result := Trim(Copy(Result, Pos(':', Result) + 1, Length(Result)));
  end;
end;

function TFPCustomHTTPClient.IndexOfHeader(const AHeader: string): Integer;
begin
  Result := IndexOfHeader(FRequestHeaders, AHeader);
end;

function TFPCustomHTTPClient.GetHeader(const AHeader: string): string;
begin
  Result := GetHeader(FRequestHeaders, AHeader);
end;

procedure TFPCustomHTTPClient.AddHeader(const AHeader, AValue: string);
var
  I: Integer;
begin
  I := IndexOfHeader(AHeader);
  if I >= 0 then
    FRequestHeaders[I] := AHeader + ': ' + AValue
  else
    FRequestHeaders.Add(AHeader + ': ' + AValue);
end;

function TFPCustomHTTPClient.ReadToBuffer: Boolean;
var
  Chunk: array[0..16383] of Char;
  N: LongInt;
  S: string;
begin
  N := FSocket.Read(Chunk, SizeOf(Chunk));
  Result := N > 0;
  if Result then
  begin
    SetLength(S, N);
    Move(Chunk, S[1], N);
    FBuffer := FBuffer + S;
  end;
end;

function TFPCustomHTTPClient.ReadLine: string;
var
  P: Integer;
begin
  repeat
    P := Pos(#10, FBuffer);
    if P > 0 then
    begin
      Result := Copy(FBuffer, 1, P - 1);
      FBuffer := Copy(FBuffer, P + 1, Length(FBuffer));
      if (Result <> '') and (Result[Length(Result)] = #13) then
        SetLength(Result, Length(Result) - 1);
      Exit;
    end;
  until not ReadToBuffer;
  Result := FBuffer;
  FBuffer := '';
end;

procedure TFPCustomHTTPClient.ReadBody(Stream: TStream);
var
  Len, ChunkLen: Int64;
  Line, Encoding: string;
  P: Integer;
begin
  Encoding := LowerCase(GetHeader(FResponseHeaders, 'Transfer-Encoding'));
  if Pos('chunked', Encoding) > 0 then
  begin
    repeat
      Line := ReadLine;
      P := Pos(';', Line);
      if P > 0 then
        Line := Copy(Line, 1, P - 1);
      ChunkLen := StrToIntDef('$' + Trim(Line), 0);
      if ChunkLen > 0 then
      begin
        while (Length(FBuffer) < ChunkLen) and ReadToBuffer do
          ;
        if Length(FBuffer) < ChunkLen then
          ChunkLen := Length(FBuffer);
        if (Stream <> nil) and (ChunkLen > 0) then
          Stream.WriteBuffer(FBuffer[1], ChunkLen);
        FBuffer := Copy(FBuffer, ChunkLen + 1, Length(FBuffer));
        ReadLine;
      end;
    until ChunkLen <= 0;
    { Trailers. }
    repeat
      Line := ReadLine;
    until Line = '';
    Exit;
  end;
  Line := GetHeader(FResponseHeaders, 'Content-Length');
  if Line <> '' then
  begin
    Len := StrToIntDef(Line, 0);
    while (Length(FBuffer) < Len) and ReadToBuffer do
      ;
    if Length(FBuffer) < Len then
      Len := Length(FBuffer);
    if (Stream <> nil) and (Len > 0) then
      Stream.WriteBuffer(FBuffer[1], Len);
    FBuffer := Copy(FBuffer, Len + 1, Length(FBuffer));
    Exit;
  end;
  { Until the server closes the connection. }
  while ReadToBuffer do
    ;
  if (Stream <> nil) and (Length(FBuffer) > 0) then
    Stream.WriteBuffer(FBuffer[1], Length(FBuffer));
  FBuffer := '';
end;

procedure TFPCustomHTTPClient.DoRequest(const AMethod, AURL: string; Stream: TStream; AllowedCode: Integer);
var
  Protocol, Host, Path, Request, Line, Location, Method, URL, HostHeader, Body: string;
  Port: Word;
  Handler: TSocketHandler;
  SSLHandler: TSSLSocketHandler;
  I, P, Redirects: Integer;
  Redirected: Boolean;
  BodyLen: Int64;
  Failure: EHTTPClient;
begin
  Method := AMethod;
  URL := AURL;
  Redirects := 0;
  repeat
    Redirected := False;
    SplitURL(URL, Protocol, Host, Path, Port);
    Handler := nil;
    if Protocol = 'https' then
    begin
      SSLHandler := TSSLSocketHandler.GetDefaultHandler;
      SSLHandler.VerifyPeerCert := FVerifySSLCertificate;
      SSLHandler.CertificateData.HostName := Host;
      Handler := SSLHandler;
    end;
    FreeAndNil(FSocket);
    FBuffer := '';
    FSocket := TInetSocket.Create(Host, Port, FConnectTimeout, Handler);
    try
      if FIOTimeout > 0 then
        FSocket.IOTimeout := FIOTimeout;
      HostHeader := Host;
      if ((Protocol = 'https') and (Port <> 443)) or ((Protocol <> 'https') and (Port <> 80)) then
        HostHeader := Host + ':' + IntToStr(Port);
      Request := Method + ' ' + Path + ' HTTP/' + FHTTPVersion + #13#10;
      if IndexOfHeader('Host') < 0 then
        Request := Request + 'Host: ' + HostHeader + #13#10;
      if IndexOfHeader('User-Agent') < 0 then
        Request := Request + 'User-Agent: Mozilla/5.0 (compatible; fpweb)' + #13#10;
      if (FUserName <> '') and (IndexOfHeader('Authorization') < 0) then
        Request := Request + 'Authorization: Basic ' + EncodeBase64(FUserName + ':' + FPassword) + #13#10;
      Body := '';
      if (FRequestBody <> nil) and (Method <> 'GET') and (Method <> 'HEAD') then
      begin
        FRequestBody.Position := 0;
        BodyLen := FRequestBody.Size;
        SetLength(Body, BodyLen);
        if BodyLen > 0 then
          FRequestBody.ReadBuffer(Body[1], BodyLen);
        if IndexOfHeader('Content-Length') < 0 then
          Request := Request + 'Content-Length: ' + IntToStr(BodyLen) + #13#10;
      end
      else if (Method = 'POST') or (Method = 'PUT') then
        if IndexOfHeader('Content-Length') < 0 then
          Request := Request + 'Content-Length: 0' + #13#10;
      for I := 0 to FRequestHeaders.Count - 1 do
        Request := Request + FRequestHeaders[I] + #13#10;
      if IndexOfHeader('Connection') < 0 then
        Request := Request + 'Connection: close' + #13#10;
      Request := Request + #13#10 + Body;
      FSocket.WriteBuffer(Request[1], Length(Request));

      { Status line: `HTTP/1.1 200 OK`. }
      Line := ReadLine;
      if Pos('HTTP/', Line) <> 1 then
        raise EHTTPClient.Create('Invalid server response: ' + Line);
      P := Pos(' ', Line);
      FServerHTTPVersion := Copy(Line, 6, P - 6);
      Line := Copy(Line, P + 1, Length(Line));
      P := Pos(' ', Line);
      if P = 0 then
        P := Length(Line) + 1;
      FResponseStatusCode := StrToIntDef(Copy(Line, 1, P - 1), 0);
      FResponseStatusText := Copy(Line, P + 1, Length(Line));
      FResponseHeaders.Clear;
      repeat
        Line := ReadLine;
        if Line <> '' then
          FResponseHeaders.Add(Line);
      until Line = '';

      if FAllowRedirect and ((FResponseStatusCode = 301) or (FResponseStatusCode = 302) or
        (FResponseStatusCode = 303) or (FResponseStatusCode = 307) or (FResponseStatusCode = 308)) then
      begin
        Location := GetHeader(FResponseHeaders, 'Location');
        if (Location <> '') and (Redirects < FMaxRedirects) then
        begin
          if Pos('://', Location) = 0 then
          begin
            if (Location <> '') and (Location[1] = '/') then
              Location := Protocol + '://' + HostHeader + Location
            else
              Location := Protocol + '://' + HostHeader + '/' + Location;
          end;
          if (FResponseStatusCode = 303) or (((FResponseStatusCode = 301) or (FResponseStatusCode = 302))
            and (Method = 'POST')) then
            Method := 'GET';
          URL := Location;
          Inc(Redirects);
          Redirected := True;
        end;
      end;
      if not Redirected then
      begin
        if Method <> 'HEAD' then
          ReadBody(Stream);
        if (AllowedCode <> 0) and (FResponseStatusCode <> AllowedCode) then
        begin
          Failure := EHTTPClient.Create('Unexpected response status code: ' +
            IntToStr(FResponseStatusCode));
          Failure.StatusCode := FResponseStatusCode;
          raise Failure;
        end;
      end;
    finally
      FreeAndNil(FSocket);
    end;
  until not Redirected;
end;

procedure TFPCustomHTTPClient.HTTPMethod(const AMethod, AURL: string; Stream: TStream; AllowedCode: Integer);
begin
  DoRequest(AMethod, AURL, Stream, AllowedCode);
end;

procedure TFPCustomHTTPClient.Get(const AURL: string; Stream: TStream);
begin
  DoRequest('GET', AURL, Stream, 200);
end;

function TFPCustomHTTPClient.Get(const AURL: string): string;
var
  S: TStringStream;
begin
  S := TStringStream.Create('');
  try
    Get(AURL, S);
    Result := S.DataString;
  finally
    S.Free;
  end;
end;

procedure TFPCustomHTTPClient.Get(const AURL: string; const LocalFileName: string);
var
  F: TFileStream;
begin
  F := TFileStream.Create(LocalFileName, fmCreate);
  try
    Get(AURL, F);
  finally
    F.Free;
  end;
end;

class function TFPCustomHTTPClient.SimpleGet(const AURL: string): string;
var
  C: TFPCustomHTTPClient;
begin
  C := Self.Create(nil);
  try
    C.AllowRedirect := True;
    Result := C.Get(AURL);
  finally
    C.Free;
  end;
end;

class procedure TFPCustomHTTPClient.SimpleGet(const AURL: string; Stream: TStream);
var
  C: TFPCustomHTTPClient;
begin
  C := Self.Create(nil);
  try
    C.AllowRedirect := True;
    C.Get(AURL, Stream);
  finally
    C.Free;
  end;
end;

procedure TFPCustomHTTPClient.Post(const URL: string; const Response: TStream);
begin
  DoRequest('POST', URL, Response, 0);
end;

function TFPCustomHTTPClient.Post(const URL: string): string;
var
  S: TStringStream;
begin
  S := TStringStream.Create('');
  try
    Post(URL, S);
    Result := S.DataString;
  finally
    S.Free;
  end;
end;

function TFPCustomHTTPClient.FormPost(const URL, FormData: string): string;
var
  Body: TStringStream;
  OldBody: TStream;
begin
  Body := TStringStream.Create(FormData);
  OldBody := FRequestBody;
  FRequestBody := Body;
  try
    if IndexOfHeader('Content-Type') < 0 then
      AddHeader('Content-Type', 'application/x-www-form-urlencoded');
    Result := Post(URL);
  finally
    FRequestBody := OldBody;
    Body.Free;
  end;
end;

class function TFPCustomHTTPClient.SimpleFormPost(const URL, FormData: string): string;
var
  C: TFPCustomHTTPClient;
begin
  C := Self.Create(nil);
  try
    Result := C.FormPost(URL, FormData);
  finally
    C.Free;
  end;
end;

function TFPCustomHTTPClient.Put(const URL: string): string;
var
  S: TStringStream;
begin
  S := TStringStream.Create('');
  try
    DoRequest('PUT', URL, S, 0);
    Result := S.DataString;
  finally
    S.Free;
  end;
end;

function TFPCustomHTTPClient.Delete(const URL: string): string;
var
  S: TStringStream;
begin
  S := TStringStream.Create('');
  try
    DoRequest('DELETE', URL, S, 0);
    Result := S.DataString;
  finally
    S.Free;
  end;
end;

procedure TFPCustomHTTPClient.Head(const AURL: string; Headers: TStrings);
begin
  DoRequest('HEAD', AURL, nil, 200);
  Headers.Assign(FResponseHeaders);
end;

end.
