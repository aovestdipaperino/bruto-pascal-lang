{ Bruto Pascal runtime: FPC's sslsockets unit. TSSLSocketHandler is the
  base of TLS socket handlers; the opensslsockets unit provides one and
  makes it the default, so `TSSLSocketHandler.GetDefaultHandler` gives a
  working handler in a program that uses it. }
unit sslsockets;

interface

uses SysUtils, Classes, ssockets, sslbase;

type
  ESSLSocketError = class(ESocketError);
  TVerifyCertificateEvent = procedure(Sender: TObject; var Allow: Boolean) of object;

  TSSLSocketHandler = class;
  TSSLSocketHandlerClass = class of TSSLSocketHandler;

  TSSLSocketHandler = class(TSocketHandler)
  private
    FCertificateData: TCertificateData;
    FVerifyPeerCert: Boolean;
    FOnVerifyCertificate: TVerifyCertificateEvent;
    FSSLType: TSSLType;
    FSSLActive: Boolean;
    FSendHostAsSNI: Boolean;
    procedure SetCertificateData(AValue: TCertificateData);
  protected
    procedure SetSSLActive(aValue: Boolean);
    { Whether to go on with a peer certificate OpenSSL accepted. }
    function DoVerifyCert: Boolean; virtual;
    function GetLastSSLErrorString: string; virtual; abstract;
    function GetLastSSLErrorCode: Integer; virtual; abstract;
  public
    constructor Create; override;
    destructor Destroy; override;
    function GetLastErrorDescription: string; override;
    function CreateClientHandler: TSocketHandler; override;
    class procedure SetDefaultHandlerClass(aClass: TSSLSocketHandlerClass);
    class function GetDefaultHandlerClass: TSSLSocketHandlerClass;
    { A new handler of the default class. }
    class function GetDefaultHandler: TSSLSocketHandler;
    function CreateCertificateData: TCertificateData; virtual;
    property SSLActive: Boolean read FSSLActive;
    property LastSSLErrorString: string read GetLastSSLErrorString;
    property LastSSLErrorCode: Integer read GetLastSSLErrorCode;
    property SSLType: TSSLType read FSSLType write FSSLType;
    { Check the server's certificate (chain and host name). }
    property VerifyPeerCert: Boolean read FVerifyPeerCert write FVerifyPeerCert;
    property SendHostAsSNI: Boolean read FSendHostAsSNI write FSendHostAsSNI;
    property CertificateData: TCertificateData read FCertificateData write SetCertificateData;
    property OnVerifyCertificate: TVerifyCertificateEvent read FOnVerifyCertificate write FOnVerifyCertificate;
  end;

implementation

var
  DefaultSSLHandlerClass: TSSLSocketHandlerClass;

constructor TSSLSocketHandler.Create;
begin
  inherited Create;
  FSendHostAsSNI := True;
  FCertificateData := CreateCertificateData;
end;

destructor TSSLSocketHandler.Destroy;
begin
  FreeAndNil(FCertificateData);
  inherited Destroy;
end;

function TSSLSocketHandler.CreateCertificateData: TCertificateData;
begin
  Result := TCertificateData.Create;
end;

procedure TSSLSocketHandler.SetCertificateData(AValue: TCertificateData);
begin
  if AValue <> FCertificateData then
    FCertificateData.Assign(AValue);
end;

procedure TSSLSocketHandler.SetSSLActive(aValue: Boolean);
begin
  FSSLActive := aValue;
end;

function TSSLSocketHandler.DoVerifyCert: Boolean;
begin
  Result := True;
  if Assigned(FOnVerifyCertificate) then
    FOnVerifyCertificate(Self, Result);
end;

function TSSLSocketHandler.CreateClientHandler: TSocketHandler;
var
  H: TSSLSocketHandler;
begin
  Result := inherited CreateClientHandler;
  if Result is TSSLSocketHandler then
  begin
    H := TSSLSocketHandler(Result);
    H.CertificateData := FCertificateData;
    H.VerifyPeerCert := FVerifyPeerCert;
    H.SendHostAsSNI := FSendHostAsSNI;
    H.SSLType := FSSLType;
  end;
end;

function TSSLSocketHandler.GetLastErrorDescription: string;
begin
  Result := GetLastSSLErrorString;
  if Result = '' then
    Result := inherited GetLastErrorDescription;
end;

class procedure TSSLSocketHandler.SetDefaultHandlerClass(aClass: TSSLSocketHandlerClass);
begin
  DefaultSSLHandlerClass := aClass;
end;

class function TSSLSocketHandler.GetDefaultHandlerClass: TSSLSocketHandlerClass;
begin
  Result := DefaultSSLHandlerClass;
end;

class function TSSLSocketHandler.GetDefaultHandler: TSSLSocketHandler;
begin
  if DefaultSSLHandlerClass = nil then
    raise ESSLSocketError.Create(seCreationFailed, 'No SSL Socket support compiled in.' + sLineBreak +
      'Please include opensslsockets unit in program and recompile it.');
  Result := DefaultSSLHandlerClass.Create;
end;

end.
