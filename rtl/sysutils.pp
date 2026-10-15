{ Bruto Pascal runtime: a small part of FPC's SysUtils unit. Format is
  built into the compiler and uses BrutoCFormat from here. }
unit SysUtils;

interface

uses UnixType;

type
  { The root of FPC's exception classes. }
  Exception = class(TObject)
  private
    FMessage: string;
    FHelpContext: LongInt;
  public
    constructor Create(const Msg: string);
    constructor CreateHelp(const Msg: string; AHelpContext: LongInt);
    { The class name and the message: `EConvertError: ...`. }
    function ToString: string; override;
    property Message: string read FMessage write FMessage;
    property HelpContext: LongInt read FHelpContext write FHelpContext;
  end;
  ExceptClass = class of Exception;

  EAbort = class(Exception);
  EExternal = class(Exception);
  EIntError = class(EExternal);
  EDivByZero = class(EIntError);
  ERangeError = class(EIntError);
  EIntOverflow = class(EIntError);
  EMathError = class(EExternal);
  EInvalidOp = class(EMathError);
  EZeroDivide = class(EMathError);
  EOverflow = class(EMathError);
  EUnderflow = class(EMathError);
  EAccessViolation = class(EExternal);
  EInOutError = class(Exception)
  public
    ErrorCode: LongInt;
  end;
  EHeapMemoryError = class(Exception);
  EOutOfMemory = class(EHeapMemoryError);
  EInvalidPointer = class(EHeapMemoryError);
  EInvalidCast = class(Exception);
  EConvertError = class(Exception);
  EAbstractError = class(Exception);
  EAssertionFailed = class(Exception);
  EOSError = class(Exception)
  public
    ErrorCode: LongInt;
  end;
  EArgumentException = class(Exception);
  EArgumentOutOfRangeException = class(EArgumentException);
  EArgumentNilException = class(EArgumentException);
  ENotImplemented = class(Exception);
  ENotSupportedException = class(Exception);

{ Raise EAbort, the exception that is handled without a message. }
procedure Abort;
{ The exception being handled, in an exception handler. }
function ExceptObject: TObject;

function IntToStr(Value: Int64): string;
function IntToHex(Value: Int64; Digits: Integer): string;
function StrToInt(const S: string): Int64;
function StrToIntDef(const S: string; Default: Int64): Int64;
function UpperCase(const S: string): string;
function LowerCase(const S: string): string;
function Trim(const S: string): string;
function TrimLeft(const S: string): string;
function TrimRight(const S: string): string;
function FileExists(const FileName: string): Boolean;
function DirectoryExists(const Directory: string): Boolean;
procedure Sleep(Milliseconds: LongWord);
{ Free the class instance Obj refers to (if any) and set Obj to nil. }
procedure FreeAndNil(var Obj);
{ A Pascal format string (`%d`, `%4.4x`, `%s`...) as the C one that
  prints the same with 64-bit integers. }
function BrutoCFormat(const Fmt: string): string;

implementation

function c_access(Path: PChar; Mode: cint): cint; cdecl; external 'c' name 'access';
{$ifdef WINDOWS}
function win_GetFileAttributesA(Path: PChar): LongWord; cdecl; external 'kernel32' name 'GetFileAttributesA';
procedure win_Sleep(Milliseconds: LongWord); cdecl; external 'kernel32' name 'Sleep';
{$else}
function c_usleep(Micros: cuint): cint; cdecl; external 'c' name 'usleep';
function c_opendir(Path: PChar): Pointer; cdecl; external 'c' name 'opendir';
function c_closedir(Dir: Pointer): cint; cdecl; external 'c' name 'closedir';
{$endif}

function IntToStr(Value: Int64): string;
var
  S: string;
begin
  Str(Value, S);
  IntToStr := S;
end;

function IntToHex(Value: Int64; Digits: Integer): string;
begin
  IntToHex := HexStr(Value, Digits);
end;

function StrToIntDef(const S: string; Default: Int64): Int64;
var
  V: Int64;
  Code: Integer;
begin
  Val(Trim(S), V, Code);
  if Code = 0 then
    StrToIntDef := V
  else
    StrToIntDef := Default;
end;

function StrToInt(const S: string): Int64;
var
  V: Int64;
  Code: Integer;
begin
  Val(Trim(S), V, Code);
  if Code <> 0 then
    raise EConvertError.Create('"' + S + '" is an invalid integer');
  StrToInt := V;
end;

function UpperCase(const S: string): string;
var
  I: Integer;
  R: string;
begin
  R := '';
  for I := 1 to Length(S) do
    if (S[I] >= 'a') and (S[I] <= 'z') then
      R := R + Chr(Ord(S[I]) - 32)
    else
      R := R + S[I];
  UpperCase := R;
end;

function LowerCase(const S: string): string;
var
  I: Integer;
  R: string;
begin
  R := '';
  for I := 1 to Length(S) do
    if (S[I] >= 'A') and (S[I] <= 'Z') then
      R := R + Chr(Ord(S[I]) + 32)
    else
      R := R + S[I];
  LowerCase := R;
end;

function TrimLeft(const S: string): string;
var
  I: Integer;
begin
  I := 1;
  while (I <= Length(S)) and (S[I] <= ' ') do
    I := I + 1;
  TrimLeft := Copy(S, I, Length(S) - I + 1);
end;

function TrimRight(const S: string): string;
var
  I: Integer;
begin
  I := Length(S);
  while (I > 0) and (S[I] <= ' ') do
    I := I - 1;
  TrimRight := Copy(S, 1, I);
end;

function Trim(const S: string): string;
begin
  Trim := TrimLeft(TrimRight(S));
end;

function FileExists(const FileName: string): Boolean;
begin
  FileExists := c_access(PChar(FileName), 0) = 0;
end;

{$ifdef WINDOWS}
function DirectoryExists(const Directory: string): Boolean;
var
  Attributes: LongWord;
begin
  Attributes := win_GetFileAttributesA(PChar(Directory));
  { INVALID_FILE_ATTRIBUTES, else FILE_ATTRIBUTE_DIRECTORY }
  DirectoryExists := (Attributes <> $FFFFFFFF) and ((Attributes and $10) <> 0);
end;

procedure Sleep(Milliseconds: LongWord);
begin
  win_Sleep(Milliseconds);
end;
{$else}
function DirectoryExists(const Directory: string): Boolean;
var
  D: Pointer;
begin
  D := c_opendir(PChar(Directory));
  DirectoryExists := D <> nil;
  if D <> nil then
    c_closedir(D);
end;

procedure Sleep(Milliseconds: LongWord);
begin
  c_usleep(Milliseconds * 1000);
end;
{$endif}

function BrutoCFormat(const Fmt: string): string;
var
  I: Integer;
  R: string;
  C: Char;
begin
  R := '';
  I := 1;
  while I <= Length(Fmt) do
  begin
    C := Fmt[I];
    R := R + C;
    I := I + 1;
    if C = '%' then
    begin
      { Flags, width and precision carry over. }
      while (I <= Length(Fmt)) and (Fmt[I] in ['-', '0'..'9', '.', '*']) do
      begin
        R := R + Fmt[I];
        I := I + 1;
      end;
      if I <= Length(Fmt) then
      begin
        case Fmt[I] of
          'd', 'D': R := R + 'lld';
          'u', 'U': R := R + 'llu';
          'x', 'X': R := R + 'llX';
          'n', 'N', 'm', 'M': R := R + 'f';
        else
          R := R + LowerCase(Fmt[I]);
        end;
        I := I + 1;
      end;
    end;
  end;
  BrutoCFormat := R;
end;

type
  { A class's destructor: Self, and whether to free the instance. }
  TBrutoDestructor = procedure(Instance: Pointer; FreeIt: Int64);

procedure FreeAndNil(var Obj);
var
  Instance, VMT: Pointer;
begin
  Instance := PPointer(@Obj)^;
  PPointer(@Obj)^ := nil;
  if Instance <> nil then
  begin
    { Destroy is the first virtual method, after the class VMT's five
      header words. }
    VMT := PPointer(Instance)^;
    TBrutoDestructor(PPointer(PtrUInt(VMT) + 5 * SizeOf(Pointer))^)(Instance, 1);
  end;
end;

constructor Exception.Create(const Msg: string);
begin
  inherited Create;
  FMessage := Msg;
end;

constructor Exception.CreateHelp(const Msg: string; AHelpContext: LongInt);
begin
  inherited Create;
  FMessage := Msg;
  FHelpContext := AHelpContext;
end;

function Exception.ToString: string;
begin
  Result := ClassName + ': ' + FMessage;
end;

procedure Abort;
begin
  raise EAbort.Create('Operation aborted');
end;

function ExceptObject: TObject;
begin
  ExceptObject := BrutoExcObject;
end;

{ Runtime errors become exceptions, as with FPC's SysUtils. }
procedure BrutoRunErrorToException(Code: LongInt);
begin
  case Code of
    200: raise EDivByZero.Create('Division by zero');
    201: raise ERangeError.Create('Range check error');
    203: raise EOutOfMemory.Create('Out of memory');
    204: raise EInvalidPointer.Create('Invalid pointer operation');
    205: raise EOverflow.Create('Floating point overflow');
    207: raise EInvalidOp.Create('Invalid floating point operation');
    210: raise EAbstractError.Create('Abstract method called');
    211: raise EAbstractError.Create('Abstract method called');
    215: raise EIntOverflow.Create('Arithmetic overflow');
    216: raise EAccessViolation.Create('Access violation');
    219: raise EInvalidCast.Create('Invalid type cast');
  end;
end;

initialization
  BrutoRunErrorHook := @BrutoRunErrorToException;
end.
