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

const
  { FileOpen modes and sharing flags (sharing is accepted and ignored). }
  fmOpenRead = $0000;
  fmOpenWrite = $0001;
  fmOpenReadWrite = $0002;
  fmShareCompat = $0000;
  fmShareExclusive = $0010;
  fmShareDenyWrite = $0020;
  fmShareDenyRead = $0030;
  fmShareDenyNone = $0040;
  { FileSeek origins. }
  fsFromBeginning = 0;
  fsFromCurrent = 1;
  fsFromEnd = 2;

{ Files by handle: -1 when they can't be opened or created. }
function FileOpen(const FileName: string; Mode: LongInt): THandle;
function FileCreate(const FileName: string): THandle;
function FileRead(Handle: THandle; var Buffer; Count: LongInt): LongInt;
function FileWrite(Handle: THandle; const Buffer; Count: LongInt): LongInt;
function FileSeek(Handle: THandle; FOffset: Int64; Origin: LongInt): Int64;
procedure FileClose(Handle: THandle);
function DeleteFile(const FileName: string): Boolean;
function RenameFile(const OldName, NewName: string): Boolean;

{ Comparisons: < 0, 0 or > 0, as FPC's. The Text ones ignore ASCII
  case. }
function CompareStr(const S1, S2: string): LongInt;
function CompareText(const S1, S2: string): LongInt;
function SameStr(const S1, S2: string): Boolean;
function SameText(const S1, S2: string): Boolean;
function AnsiCompareStr(const S1, S2: string): LongInt;
function AnsiCompareText(const S1, S2: string): LongInt;
function AnsiSameText(const S1, S2: string): Boolean;
function AnsiUpperCase(const S: string): string;
function AnsiLowerCase(const S: string): string;
function QuotedStr(const S: string): string;
function AnsiQuotedStr(const S: string; Quote: Char): string;
function BoolToStr(B: Boolean; UseBoolStrs: Boolean): string;

{ The error code of the last system call that failed (errno), and its
  description. }
function GetLastOSError: LongInt;
function SysErrorMessage(ErrorCode: LongInt): string;

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

{$ifdef WINDOWS}
function bruto_open(Path: PChar; Flags: LongInt): LongInt; cdecl; varargs; external 'c' name '_open';
function bruto_read(Fd: LongInt; Buf: Pointer; Count: LongWord): LongInt; cdecl; external 'c' name '_read';
function bruto_write(Fd: LongInt; Buf: Pointer; Count: LongWord): LongInt; cdecl; external 'c' name '_write';
function bruto_lseek(Fd: LongInt; Offset: Int64; Origin: LongInt): Int64; cdecl; external 'c' name '_lseeki64';
function bruto_close(Fd: LongInt): LongInt; cdecl; external 'c' name '_close';
function bruto_unlink(Path: PChar): LongInt; cdecl; external 'c' name '_unlink';
const
  BrutoOpenFlags: array[0..2] of LongInt = ($8000, $8001, $8002);
  BrutoCreateFlags = $8000 or $0001 or $0100 or $0200;
{$else}
{ open's mode is a variadic argument (passed on the stack on arm64). }
function bruto_open(Path: PChar; Flags: LongInt): LongInt; cdecl; varargs; external 'c' name 'open';
function bruto_read(Fd: LongInt; Buf: Pointer; Count: PtrUInt): PtrInt; cdecl; external 'c' name 'read';
function bruto_write(Fd: LongInt; Buf: Pointer; Count: PtrUInt): PtrInt; cdecl; external 'c' name 'write';
function bruto_lseek(Fd: LongInt; Offset: Int64; Origin: LongInt): Int64; cdecl; external 'c' name 'lseek';
function bruto_close(Fd: LongInt): LongInt; cdecl; external 'c' name 'close';
function bruto_unlink(Path: PChar): LongInt; cdecl; external 'c' name 'unlink';
const
  BrutoOpenFlags: array[0..2] of LongInt = (0, 1, 2);
{$ifdef DARWIN}
  BrutoCreateFlags = 1 or $0200 or $0400;
{$else}
  BrutoCreateFlags = 1 or $0040 or $0200;
{$endif}
{$endif}
function bruto_rename(OldPath, NewPath: PChar): LongInt; cdecl; external 'c' name 'rename';
function bruto_strerror(Code: LongInt): PChar; cdecl; external 'c' name 'strerror';
{$ifdef WINDOWS}
function bruto_errno_location: PLongInt; cdecl; external 'c' name '_errno';
{$else}
{$ifdef DARWIN}
function bruto_errno_location: PLongInt; cdecl; external 'c' name '__error';
{$else}
function bruto_errno_location: PLongInt; cdecl; external 'c' name '__errno_location';
{$endif}
{$endif}

function GetLastOSError: LongInt;
var
  P: PLongInt;
begin
  P := bruto_errno_location;
  GetLastOSError := P^;
end;

function SysErrorMessage(ErrorCode: LongInt): string;
var
  S: string;
begin
  S := bruto_strerror(ErrorCode);
  SysErrorMessage := S;
end;

function FileOpen(const FileName: string; Mode: LongInt): THandle;
begin
  FileOpen := bruto_open(PChar(FileName), BrutoOpenFlags[Mode and 3]);
end;

function FileCreate(const FileName: string): THandle;
begin
  FileCreate := bruto_open(PChar(FileName), BrutoCreateFlags, LongInt(438));
end;

function FileRead(Handle: THandle; var Buffer; Count: LongInt): LongInt;
begin
  FileRead := bruto_read(Handle, @Buffer, Count);
end;

function FileWrite(Handle: THandle; const Buffer; Count: LongInt): LongInt;
begin
  FileWrite := bruto_write(Handle, @Buffer, Count);
end;

function FileSeek(Handle: THandle; FOffset: Int64; Origin: LongInt): Int64;
begin
  FileSeek := bruto_lseek(Handle, FOffset, Origin);
end;

procedure FileClose(Handle: THandle);
begin
  bruto_close(Handle);
end;

function DeleteFile(const FileName: string): Boolean;
begin
  DeleteFile := bruto_unlink(PChar(FileName)) = 0;
end;

function RenameFile(const OldName, NewName: string): Boolean;
begin
  RenameFile := bruto_rename(PChar(OldName), PChar(NewName)) = 0;
end;

function CompareStr(const S1, S2: string): LongInt;
var
  I, N: Integer;
begin
  N := Length(S1);
  if Length(S2) < N then
    N := Length(S2);
  for I := 1 to N do
    if S1[I] <> S2[I] then
    begin
      if S1[I] < S2[I] then
        Exit(-1);
      Exit(1);
    end;
  CompareStr := Length(S1) - Length(S2);
end;

function CompareText(const S1, S2: string): LongInt;
var
  I, N, C1, C2: Integer;
begin
  N := Length(S1);
  if Length(S2) < N then
    N := Length(S2);
  for I := 1 to N do
  begin
    C1 := Ord(S1[I]);
    C2 := Ord(S2[I]);
    if C1 <> C2 then
    begin
      if (C1 >= 97) and (C1 <= 122) then
        C1 := C1 - 32;
      if (C2 >= 97) and (C2 <= 122) then
        C2 := C2 - 32;
      if C1 <> C2 then
        Exit(C1 - C2);
    end;
  end;
  CompareText := Length(S1) - Length(S2);
end;

function SameStr(const S1, S2: string): Boolean;
begin
  SameStr := CompareStr(S1, S2) = 0;
end;

function SameText(const S1, S2: string): Boolean;
begin
  SameText := CompareText(S1, S2) = 0;
end;

function AnsiCompareStr(const S1, S2: string): LongInt;
begin
  AnsiCompareStr := CompareStr(S1, S2);
end;

function AnsiCompareText(const S1, S2: string): LongInt;
begin
  AnsiCompareText := CompareStr(LowerCase(S1), LowerCase(S2));
  if AnsiCompareText < 0 then
    AnsiCompareText := -1
  else if AnsiCompareText > 0 then
    AnsiCompareText := 1;
end;

function AnsiSameText(const S1, S2: string): Boolean;
begin
  AnsiSameText := AnsiCompareText(S1, S2) = 0;
end;

function AnsiUpperCase(const S: string): string;
begin
  AnsiUpperCase := UpperCase(S);
end;

function AnsiLowerCase(const S: string): string;
begin
  AnsiLowerCase := LowerCase(S);
end;

function AnsiQuotedStr(const S: string; Quote: Char): string;
var
  I: Integer;
  R: string;
begin
  R := Quote;
  for I := 1 to Length(S) do
  begin
    R := R + S[I];
    if S[I] = Quote then
      R := R + Quote;
  end;
  AnsiQuotedStr := R + Quote;
end;

function QuotedStr(const S: string): string;
begin
  QuotedStr := AnsiQuotedStr(S, '''');
end;

function BoolToStr(B: Boolean; UseBoolStrs: Boolean): string;
begin
  if UseBoolStrs then
  begin
    if B then BoolToStr := 'True' else BoolToStr := 'False';
  end
  else if B then
    BoolToStr := '-1'
  else
    BoolToStr := '0';
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
