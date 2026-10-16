{ Bruto Pascal runtime: a small part of FPC's SysUtils unit. Format is
  built into the compiler and uses BrutoCFormat from here. }
unit SysUtils;

{$mode objfpc}{$H+}

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

type
  TFloatFormat = (ffGeneral, ffExponent, ffFixed, ffNumber, ffCurrency);

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

function IntToStr(Value: Int64): string; overload;
function IntToStr(Value: QWord): string; overload;
function UIntToStr(Value: QWord): string;
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
{ FPC's Format: `%[index:][-][width][.precision]type`, with types d, u,
  x, s, f, e, g, n, m and p; `*` takes a width or precision from Args. }
function Format(const Fmt: string; const Args: array of const): string;
function FloatToStr(Value: Double): string;
function FloatToStrF(Value: Double; Format: TFloatFormat; Precision, Digits: Integer): string;

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

function IntToStr(Value: Int64): string; overload;
var
  S: string;
begin
  Str(Value, S);
  IntToStr := S;
end;

function IntToStr(Value: QWord): string; overload;
var
  S: string;
begin
  Str(Value, S);
  IntToStr := S;
end;

function UIntToStr(Value: QWord): string;
var
  S: string;
begin
  Str(Value, S);
  UIntToStr := S;
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
    TBrutoDestructor(PPointer(PtrUInt(VMT) + 7 * SizeOf(Pointer))^)(Instance, 1);
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

function bruto_fmt_snprintf(Buf: PChar; Size: PtrUInt; Fmt: PChar): LongInt; cdecl; varargs;
  external 'c' name 'snprintf';

{ Value's first 15 significant digits (Digits) and its decimal exponent:
  Value = 0.D1D2... * 10^(Exp + 1), as in 1.5 = '150000000000000', 0. }
procedure FloatDigits(Value: Double; var Digits: string; var Exp: Integer; var Negative: Boolean);
var
  Buf: array[0..63] of Char;
  S: string;
  P, Code: Integer;
begin
  Negative := Value < 0;
  if Negative then
    Value := -Value;
  if Value = 0 then
  begin
    Digits := '000000000000000';
    Exp := 0;
    Exit;
  end;
  bruto_fmt_snprintf(@Buf, SizeOf(Buf), '%.14e', Value);
  S := PChar(@Buf);
  P := Pos('e', S);
  Digits := Copy(S, 1, 1) + Copy(S, 3, P - 3);
  Val(Copy(S, P + 1, Length(S)), Exp, Code);
end;

{ Round the digit string D (with exponent Exp) to Keep digits, half up. }
procedure RoundDigits(var D: string; var Exp: Integer; Keep: Integer);
var
  I: Integer;
  Carry: Boolean;
begin
  if Keep >= Length(D) then
  begin
    while Length(D) < Keep do
      D := D + '0';
    Exit;
  end;
  if Keep < 0 then
  begin
    D := '';
    Exit;
  end;
  Carry := D[Keep + 1] >= '5';
  D := Copy(D, 1, Keep);
  I := Keep;
  while Carry and (I >= 1) do
  begin
    if D[I] = '9' then
    begin
      D[I] := '0';
      I := I - 1;
    end
    else
    begin
      D[I] := Chr(Ord(D[I]) + 1);
      Carry := False;
    end;
  end;
  if Carry then
  begin
    D := '1' + D;
    Exp := Exp + 1;
    if Keep > 0 then
      D := Copy(D, 1, Keep);
  end;
end;

function GroupThousands(const IntPart: string): string;
var
  I, N: Integer;
  R: string;
begin
  R := '';
  N := 0;
  for I := Length(IntPart) downto 1 do
  begin
    if (N > 0) and (N mod 3 = 0) then
      R := ',' + R;
    R := IntPart[I] + R;
    N := N + 1;
  end;
  GroupThousands := R;
end;

{ Value with Decimals digits after the point (ffFixed), grouped in
  thousands for ffNumber. }
function FormatFixed(Value: Double; Decimals: Integer; Grouped: Boolean): string;
var
  D, IntPart, Frac: string;
  Exp, Keep: Integer;
  Negative: Boolean;
begin
  FloatDigits(Value, D, Exp, Negative);
  Keep := Exp + 1 + Decimals;
  if Value = 0 then
    Keep := 0;
  RoundDigits(D, Exp, Keep);
  if Keep <= 0 then
  begin
    if D = '' then
      D := '0'
    else
      Exp := Exp;
  end;
  { Digits before the point, then after it. }
  if D = '' then
  begin
    IntPart := '0';
    Frac := StringOfChar('0', Decimals);
  end
  else if Exp >= 0 then
  begin
    while Length(D) < Exp + 1 + Decimals do
      D := D + '0';
    IntPart := Copy(D, 1, Exp + 1);
    Frac := Copy(D, Exp + 2, Decimals);
  end
  else
  begin
    IntPart := '0';
    Frac := StringOfChar('0', -Exp - 1) + D;
    while Length(Frac) < Decimals do
      Frac := Frac + '0';
    Frac := Copy(Frac, 1, Decimals);
  end;
  if Grouped then
    IntPart := GroupThousands(IntPart);
  Result := IntPart;
  if Decimals > 0 then
    Result := Result + '.' + Frac;
  if Negative and (Pos('1', D) + Pos('2', D) + Pos('3', D) + Pos('4', D) + Pos('5', D) +
    Pos('6', D) + Pos('7', D) + Pos('8', D) + Pos('9', D) > 0) then
    Result := '-' + Result;
end;

{ Value in scientific notation with Precision significant digits and a
  3-digit exponent: 1.50000000000000E+000. }
function FormatExponent(Value: Double; Precision: Integer): string;
var
  D, ExpText: string;
  Exp: Integer;
  Negative: Boolean;
begin
  if Precision < 1 then
    Precision := 1;
  FloatDigits(Value, D, Exp, Negative);
  RoundDigits(D, Exp, Precision);
  if Value = 0 then
    Exp := 0;
  Result := D[1];
  if Length(D) > 1 then
    Result := Result + '.' + Copy(D, 2, Length(D));
  ExpText := IntToStr(Abs(Exp));
  while Length(ExpText) < 3 do
    ExpText := '0' + ExpText;
  if Exp < 0 then
    Result := Result + 'E-' + ExpText
  else
    Result := Result + 'E+' + ExpText;
  if Negative then
    Result := '-' + Result;
end;

{ The shortest of fixed and scientific notation, with at most Precision
  significant digits: 1.5, 100, 1E20, -1.5E-7. }
function FormatGeneral(Value: Double; Precision: Integer): string;
var
  D: string;
  Exp, I: Integer;
  Negative: Boolean;
begin
  if (Precision < 1) or (Precision > 15) then
    Precision := 15;
  if Value = 0 then
    Exit('0');
  FloatDigits(Value, D, Exp, Negative);
  RoundDigits(D, Exp, Precision);
  { Trailing zeros go. }
  I := Length(D);
  while (I > 1) and (D[I] = '0') do
    I := I - 1;
  D := Copy(D, 1, I);
  if (Exp < -4) or (Exp >= Precision) then
  begin
    Result := D[1];
    if Length(D) > 1 then
      Result := Result + '.' + Copy(D, 2, Length(D));
    Result := Result + 'E' + IntToStr(Exp);
  end
  else if Exp < 0 then
    Result := '0.' + StringOfChar('0', -Exp - 1) + D
  else if Length(D) <= Exp + 1 then
    Result := D + StringOfChar('0', Exp + 1 - Length(D))
  else
    Result := Copy(D, 1, Exp + 1) + '.' + Copy(D, Exp + 2, Length(D));
  if Negative then
    Result := '-' + Result;
end;

function FloatToStrF(Value: Double; Format: TFloatFormat; Precision, Digits: Integer): string;
begin
  case Format of
    ffExponent: Result := FormatExponent(Value, Precision);
    ffFixed: Result := FormatFixed(Value, Digits, False);
    ffNumber, ffCurrency: Result := FormatFixed(Value, Digits, True);
  else
    Result := FormatGeneral(Value, Precision);
  end;
end;

function FloatToStr(Value: Double): string;
begin
  FloatToStr := FormatGeneral(Value, 15);
end;

function Format(const Fmt: string; const Args: array of const): string;
var
  I, Len, ArgIndex, Index, Width, Precision, N: Integer;
  LeftAlign: Boolean;
  Kind: Char;
  Value, Number: string;
  Arg: TVarRec;
  IntValue: Int64;
  FloatValue: Double;

  procedure Fail;
  begin
    raise EConvertError.Create('Invalid argument index in format "' + Fmt + '"');
  end;

  function NextArg: TVarRec;
  begin
    if ArgIndex > High(Args) then
      Fail;
    Result := Args[ArgIndex];
    ArgIndex := ArgIndex + 1;
  end;

  { An integer from the format, or `*` taking one from Args. }
  function ReadNumber: Integer;
  var
    A: TVarRec;
  begin
    Result := -1;
    if (I <= Len) and (Fmt[I] = '*') then
    begin
      A := NextArg;
      if A.VType <> vtInteger then
        Fail;
      Result := A.VInteger;
      I := I + 1;
      Exit;
    end;
    while (I <= Len) and (Fmt[I] >= '0') and (Fmt[I] <= '9') do
    begin
      if Result < 0 then
        Result := 0;
      Result := Result * 10 + Ord(Fmt[I]) - Ord('0');
      I := I + 1;
    end;
  end;

  function ArgInteger(const A: TVarRec): Int64;
  begin
    case A.VType of
      vtInteger: Result := A.VInteger;
      vtInt64: Result := A.VInt64^;
      vtQWord: Result := Int64(A.VQWord^);
    else
      Fail;
    end;
  end;

  function ArgFloat(const A: TVarRec): Double;
  begin
    case A.VType of
      vtExtended: Result := A.VExtended^;
    else
      Fail;
    end;
  end;

  function ArgString(const A: TVarRec): string;
  begin
    case A.VType of
      vtAnsiString: Result := AnsiString(A.VAnsiString);
      vtChar: Result := A.VChar;
      vtString: Result := A.VString^;
      vtPChar: Result := A.VPChar;
      vtUnicodeString: Result := UnicodeString(A.VUnicodeString);
      vtWideChar: Result := UnicodeString(A.VWideChar);
    else
      Fail;
    end;
  end;

begin
  Result := '';
  Len := Length(Fmt);
  ArgIndex := 0;
  I := 1;
  while I <= Len do
  begin
    if Fmt[I] <> '%' then
    begin
      Result := Result + Fmt[I];
      I := I + 1;
      Continue;
    end;
    I := I + 1;
    if (I <= Len) and (Fmt[I] = '%') then
    begin
      Result := Result + '%';
      I := I + 1;
      Continue;
    end;
    { [index:] }
    Index := ReadNumber;
    if (I <= Len) and (Fmt[I] = ':') then
    begin
      ArgIndex := Index;
      I := I + 1;
      Index := -1;
    end;
    LeftAlign := False;
    if (I <= Len) and (Fmt[I] = '-') then
    begin
      LeftAlign := True;
      I := I + 1;
    end;
    if Index >= 0 then
      Width := Index
    else
      Width := ReadNumber;
    Precision := -1;
    if (I <= Len) and (Fmt[I] = '.') then
    begin
      I := I + 1;
      Precision := ReadNumber;
      if Precision < 0 then
        Precision := 0;
    end;
    if I > Len then
      Fail;
    Kind := UpCase(Fmt[I]);
    I := I + 1;
    Arg := NextArg;
    case Kind of
      'D':
        begin
          IntValue := ArgInteger(Arg);
          Number := IntToStr(Abs(IntValue));
          if IntValue = Low(Int64) then
            Number := '9223372036854775808';
          if Precision > Length(Number) then
            Number := StringOfChar('0', Precision - Length(Number)) + Number;
          if IntValue < 0 then
            Number := '-' + Number;
          Value := Number;
        end;
      'U':
        begin
          IntValue := ArgInteger(Arg);
          if (Arg.VType = vtInteger) and (IntValue < 0) then
            IntValue := IntValue + 4294967296;
          Value := IntToStr(IntValue);
          if Precision > Length(Value) then
            Value := StringOfChar('0', Precision - Length(Value)) + Value;
        end;
      'X':
        begin
          IntValue := ArgInteger(Arg);
          if (Arg.VType = vtInteger) and (IntValue < 0) then
            Value := IntToHex(IntValue and $FFFFFFFF, 16)
          else
            Value := IntToHex(IntValue, 16);
          while (Length(Value) > 1) and (Value[1] = '0') do
            Delete(Value, 1, 1);
          if Precision > Length(Value) then
            Value := StringOfChar('0', Precision - Length(Value)) + Value;
        end;
      'S':
        begin
          Value := ArgString(Arg);
          if (Precision >= 0) and (Precision < Length(Value)) then
            Value := Copy(Value, 1, Precision);
        end;
      'F', 'N', 'M':
        begin
          FloatValue := ArgFloat(Arg);
          if Precision < 0 then
            Precision := 2;
          Value := FormatFixed(FloatValue, Precision, Kind <> 'F');
        end;
      'E':
        begin
          FloatValue := ArgFloat(Arg);
          if Precision < 0 then
            Precision := 15;
          Value := FormatExponent(FloatValue, Precision);
        end;
      'G':
        begin
          FloatValue := ArgFloat(Arg);
          if Precision < 0 then
            Precision := 15;
          Value := FormatGeneral(FloatValue, Precision);
        end;
      'P':
        begin
          if Arg.VType <> vtPointer then
            Fail;
          Value := IntToHex(PtrUInt(Arg.VPointer), 16);
        end;
    else
      raise EConvertError.Create('Invalid format specifier : "' + Fmt + '"');
    end;
    N := Width - Length(Value);
    if N > 0 then
    begin
      if LeftAlign then
        Value := Value + StringOfChar(' ', N)
      else
        Value := StringOfChar(' ', N) + Value;
    end;
    Result := Result + Value;
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
    206: raise EUnderflow.Create('Floating point underflow');
    207: raise EInvalidOp.Create('Invalid floating point operation');
    208: raise EZeroDivide.Create('Floating point division by zero');
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
