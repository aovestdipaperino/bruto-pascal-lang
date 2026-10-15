{ Bruto Pascal runtime: FPC's sslbase unit, the certificate settings an
  SSL socket handler uses. Certificates and keys are given as PEM files
  (FileName) or PEM text (Value). }
unit sslbase;

interface

uses SysUtils, Classes;

type
  TSSLType = (stAny, stSSLv2, stSSLv3, stTLSv1, stTLSv1_1, stTLSv1_2);

  TSSLData = class(TPersistent)
  private
    FFileName: string;
    FValue: string;
  public
    function Empty: Boolean;
    procedure Clear;
    procedure Assign(Source: TPersistent); override;
    property FileName: string read FFileName write FFileName;
    { The PEM text itself, when there is no file. }
    property Value: string read FValue write FValue;
  end;

  TCertificateData = class(TPersistent)
  private
    FCertificate: TSSLData;
    FPrivateKey: TSSLData;
    FTrustedCertificate: TSSLData;
    FCertCA: TSSLData;
    FPFX: TSSLData;
    FKeyPassword: string;
    FCipherList: string;
    FHostName: string;
  public
    constructor Create;
    destructor Destroy; override;
    procedure Assign(Source: TPersistent); override;
    function NeedCertificateData: Boolean;
    property Certificate: TSSLData read FCertificate;
    property PrivateKey: TSSLData read FPrivateKey;
    property TrustedCertificate: TSSLData read FTrustedCertificate;
    property CertCA: TSSLData read FCertCA;
    property PFX: TSSLData read FPFX;
    property KeyPassword: string read FKeyPassword write FKeyPassword;
    property CipherList: string read FCipherList write FCipherList;
    property HostName: string read FHostName write FHostName;
  end;

implementation

function TSSLData.Empty: Boolean;
begin
  Result := (FFileName = '') and (FValue = '');
end;

procedure TSSLData.Clear;
begin
  FFileName := '';
  FValue := '';
end;

procedure TSSLData.Assign(Source: TPersistent);
begin
  if Source is TSSLData then
  begin
    FFileName := TSSLData(Source).FFileName;
    FValue := TSSLData(Source).FValue;
  end
  else
    inherited Assign(Source);
end;

constructor TCertificateData.Create;
begin
  inherited Create;
  FCertificate := TSSLData.Create;
  FPrivateKey := TSSLData.Create;
  FTrustedCertificate := TSSLData.Create;
  FCertCA := TSSLData.Create;
  FPFX := TSSLData.Create;
  FCipherList := 'DEFAULT';
end;

destructor TCertificateData.Destroy;
begin
  FCertificate.Free;
  FPrivateKey.Free;
  FTrustedCertificate.Free;
  FCertCA.Free;
  FPFX.Free;
  inherited Destroy;
end;

procedure TCertificateData.Assign(Source: TPersistent);
var
  Other: TCertificateData;
begin
  if Source is TCertificateData then
  begin
    Other := TCertificateData(Source);
    FCertificate.Assign(Other.FCertificate);
    FPrivateKey.Assign(Other.FPrivateKey);
    FTrustedCertificate.Assign(Other.FTrustedCertificate);
    FCertCA.Assign(Other.FCertCA);
    FPFX.Assign(Other.FPFX);
    FKeyPassword := Other.FKeyPassword;
    FCipherList := Other.FCipherList;
    FHostName := Other.FHostName;
  end
  else
    inherited Assign(Source);
end;

function TCertificateData.NeedCertificateData: Boolean;
begin
  Result := FCertificate.Empty and FPFX.Empty;
end;

end.
