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
  EVariantError = class(Exception)
    ErrCode: LongInt;
    constructor CreateCode(Code: LongInt);
  end;
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
function StrToInt64(const S: string): Int64;
function StrToInt64Def(const S: string; Default: Int64): Int64;
function TryStrToInt(const S: string; out Value: LongInt): Boolean;
function TryStrToInt64(const S: string; out Value: Int64): Boolean;
{ Whether the Length bytes at P1 and P2 are the same. }
function CompareMem(P1, P2: Pointer; Length: PtrUInt): Boolean;
function StrToFloat(const S: string): Double;
function StrToFloatDef(const S: string; const Default: Double): Double;
function TryStrToFloat(const S: string; out Value: Double): Boolean;
function UpperCase(const S: string): string;
function LowerCase(const S: string): string;
function Trim(const S: string): string;
function TrimLeft(const S: string): string;
function TrimRight(const S: string): string;
function FileExists(const FileName: string): Boolean;
{ File names, as FPC splits them ('/' separates directories, and '\' and
  ':' too on Windows; a name starting with '.' has no extension). }
function ChangeFileExt(const FileName, Extension: string): string;
function ExtractFileName(const FileName: string): string;
function ExtractFilePath(const FileName: string): string;
function ExtractFileDir(const FileName: string): string;
function ExtractFileExt(const FileName: string): string;
function IncludeTrailingPathDelimiter(const Path: string): string;
function ExcludeTrailingPathDelimiter(const Path: string): string;
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
function CurrToStr(Value: Currency): string;
function StrToCurr(const S: string): Currency;
function TryStrToCurr(const S: string; out Value: Currency): Boolean;
function FloatToStrF(Value: Double; Format: TFloatFormat; Precision, Digits: Integer): string;

{ ── Dates and times (FPC's dati.inc) ─────────────────────────── }

type
  { Days since 30 December 1899, the time of day as the fraction. }
  TDateTime = Double;
  TDate = TDateTime;
  TTime = TDateTime;
  PDateTime = ^TDateTime;

  TTimeStamp = record
    Time: LongInt;
    Date: LongInt;
  end;

  TSystemTime = record
    Year, Month, Day, DayOfWeek: Word;
    Hour, Minute, Second, MilliSecond: Word;
  end;

  TDayTable = array[1..12] of Word;
  PDayTable = ^TDayTable;

  TFormatDateTimeOption = (fdoInterval);
  TFormatDateTimeOptions = set of TFormatDateTimeOption;

  { FPC's TFormatSettings; DefaultFormatSettings holds FPC's defaults on
    Unix (not read from the locale). }
  TFormatSettings = record
    CurrencyFormat: Byte;
    NegCurrFormat: Byte;
    ThousandSeparator: Char;
    DecimalSeparator: Char;
    CurrencyDecimals: Byte;
    DateSeparator: Char;
    TimeSeparator: Char;
    ListSeparator: Char;
    CurrencyString: string;
    ShortDateFormat: string;
    LongDateFormat: string;
    TimeAMString: string;
    TimePMString: string;
    ShortTimeFormat: string;
    LongTimeFormat: string;
    ShortMonthNames: array[1..12] of string;
    LongMonthNames: array[1..12] of string;
    ShortDayNames: array[1..7] of string;
    LongDayNames: array[1..7] of string;
    TwoDigitYearCenturyWindow: Word;
  end;

const
  HoursPerDay = 24;
  MinsPerHour = 60;
  SecsPerMin = 60;
  MSecsPerSec = 1000;
  MinsPerDay = HoursPerDay * MinsPerHour;
  SecsPerHour = SecsPerMin * MinsPerHour;
  SecsPerDay = MinsPerDay * SecsPerMin;
  MSecsPerDay = SecsPerDay * MSecsPerSec;
  { Days between 1 January 0001 and 31 December 1899. }
  DateDelta = 693594;
  UnixDateDelta = 25569;
  JulianEpoch: TDateTime = -2415018.5;
  UnixEpoch: TDateTime = 25569.0;
  MinDateTime: TDateTime = -693593.0;
  MaxDateTime: TDateTime = 2958465.99999999;
  MonthDays: array[Boolean] of TDayTable =
    ((31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31),
     (31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31));

var
  DefaultFormatSettings: TFormatSettings;

function ComposeDateTime(Date, Time: TDateTime): TDateTime;
function DateTimeToTimeStamp(DateTime: TDateTime): TTimeStamp;
function TimeStampToDateTime(const TimeStamp: TTimeStamp): TDateTime;
function MSecsToTimeStamp(MSecs: Int64): TTimeStamp;
function TimeStampToMSecs(const TimeStamp: TTimeStamp): Int64;
function TryEncodeDate(Year, Month, Day: Word; out Date: TDateTime): Boolean;
function TryEncodeTime(Hour, Min, Sec, MSec: Word; out Time: TDateTime): Boolean;
function EncodeDate(Year, Month, Day: Word): TDateTime;
function EncodeTime(Hour, Minute, Second, MilliSecond: Word): TDateTime;
procedure DecodeDate(Date: TDateTime; out Year, Month, Day: Word);
function DecodeDateFully(const DateTime: TDateTime; out Year, Month, Day, DOW: Word): Boolean;
procedure DecodeTime(Time: TDateTime; out Hour, Minute, Second, MilliSecond: Word);
procedure DateTimeToSystemTime(DateTime: TDateTime; out SystemTime: TSystemTime);
function SystemTimeToDateTime(const SystemTime: TSystemTime): TDateTime;
procedure GetLocalTime(var SystemTime: TSystemTime);
function DayOfWeek(DateTime: TDateTime): Integer;
function Date: TDateTime;
function Time: TDateTime;
function Now: TDateTime;
function CurrentYear: Word;
function IncMonth(const DateTime: TDateTime; NumberOfMonths: Integer = 1): TDateTime;
procedure IncAMonth(var Year, Month, Day: Word; NumberOfMonths: Integer = 1);
function IsLeapYear(Year: Word): Boolean;
function DateToStr(Date: TDateTime): string;
function TimeToStr(Time: TDateTime): string;
function DateTimeToStr(DateTime: TDateTime; ForceTimeIfZero: Boolean = False): string;
function StrToDate(const S: string): TDateTime; overload;
function StrToDate(const S: string; Separator: Char): TDateTime; overload;
function StrToDate(const S: string; const UseFormat: string; Separator: Char): TDateTime; overload;
function StrToTime(const S: string): TDateTime; overload;
function StrToTime(const S: string; Separator: Char): TDateTime; overload;
function StrToDateTime(const S: string): TDateTime;
function TryStrToDate(const S: string; out Value: TDateTime): Boolean;
function TryStrToTime(const S: string; out Value: TDateTime): Boolean;
function TryStrToDateTime(const S: string; out Value: TDateTime): Boolean;
function StrToDateDef(const S: string; const DefValue: TDateTime): TDateTime;
function StrToTimeDef(const S: string; const DefValue: TDateTime): TDateTime;
function StrToDateTimeDef(const S: string; const DefValue: TDateTime): TDateTime;
function FormatDateTime(const FormatStr: string; DateTime: TDateTime;
  Options: TFormatDateTimeOptions = []): string;
procedure DateTimeToString(out Result: string; const FormatStr: string;
  const DateTime: TDateTime; Options: TFormatDateTimeOptions = []);
procedure ReplaceTime(var DateTime: TDateTime; NewTime: TDateTime);
procedure ReplaceDate(var DateTime: TDateTime; const NewDate: TDateTime);
{ A Unix file time (seconds since 1970, UTC) as local date and time, and
  back. }
function FileDateToDateTime(FileDate: Int64): TDateTime;
function DateTimeToFileDate(DateTime: TDateTime): Int64;
{ Minutes to add to local time to get UTC (negative east of Greenwich),
  as FPC's. }
function GetLocalTimeOffset: Integer;

{ ── Null-terminated strings (FPC's PChar routines) ───────────── }

function StrLen(P: PChar): SizeInt;
function StrEnd(P: PChar): PChar;
function StrMove(Dest, Source: PChar; L: SizeInt): PChar;
function StrCopy(Dest, Source: PChar): PChar;
function StrLCopy(Dest, Source: PChar; MaxLen: SizeInt): PChar;
function StrECopy(Dest, Source: PChar): PChar;
function StrPCopy(Dest: PChar; const Source: string): PChar;
function StrPLCopy(Dest: PChar; const Source: string; MaxLen: SizeInt): PChar;
function StrCat(Dest, Source: PChar): PChar;
function StrLCat(Dest, Source: PChar; L: SizeInt): PChar;
function StrComp(Str1, Str2: PChar): SizeInt;
function StrIComp(Str1, Str2: PChar): SizeInt;
function StrLComp(Str1, Str2: PChar; L: SizeInt): SizeInt;
function StrLIComp(Str1, Str2: PChar; L: SizeInt): SizeInt;
function StrScan(P: PChar; C: Char): PChar;
function StrRScan(P: PChar; C: Char): PChar;
function StrPos(Str1, Str2: PChar): PChar;
function StrUpper(P: PChar): PChar;
function StrLower(P: PChar): PChar;
function StrPas(P: PChar): string;
function StrAlloc(Size: SizeInt): PChar;
function StrNew(P: PChar): PChar;
procedure StrDispose(P: PChar);
function AnsiStrComp(S1, S2: PChar): SizeInt;
function AnsiStrIComp(S1, S2: PChar): SizeInt;
function AnsiStrLComp(S1, S2: PChar; MaxLen: SizeInt): SizeInt;
function AnsiStrLIComp(S1, S2: PChar; MaxLen: SizeInt): SizeInt;

type
  TSysCharSet = set of AnsiChar;
  PSysCharSet = ^TSysCharSet;

function CharInSet(C: AnsiChar; const CheckSet: TSysCharSet): Boolean; overload;
function CharInSet(C: WideChar; const CheckSet: TSysCharSet): Boolean; overload;

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

function StrToInt64(const S: string): Int64;
begin
  StrToInt64 := StrToInt(S);
end;

function StrToInt64Def(const S: string; Default: Int64): Int64;
begin
  StrToInt64Def := StrToIntDef(S, Default);
end;

function TryStrToInt64(const S: string; out Value: Int64): Boolean;
var
  Code: Integer;
begin
  Val(S, Value, Code);
  Result := Code = 0;
  if not Result then
    Value := 0;
end;

{ As FPC's (Val into a LongInt): a value beyond LongInt wraps. }
function TryStrToInt(const S: string; out Value: LongInt): Boolean;
var
  V: Int64;
begin
  Result := TryStrToInt64(S, V);
  Value := LongInt(V);
end;

function CompareMem(P1, P2: Pointer; Length: PtrUInt): Boolean;
var
  I: PtrUInt;
begin
  CompareMem := True;
  if Length = 0 then
    Exit;
  for I := 0 to Length - 1 do
    if PByte(P1 + I)^ <> PByte(P2 + I)^ then
    begin
      CompareMem := False;
      Exit;
    end;
end;

function TryStrToFloat(const S: string; out Value: Double): Boolean;
var
  Code: Integer;
  V: Double;
begin
  Val(Trim(S), V, Code);
  TryStrToFloat := (Code = 0) and (Trim(S) <> '');
  if Code = 0 then
    Value := V
  else
    Value := 0;
end;

function StrToFloat(const S: string): Double;
var
  V: Double;
begin
  if not TryStrToFloat(S, V) then
    raise EConvertError.Create('"' + S + '" is an invalid float');
  StrToFloat := V;
end;

function StrToFloatDef(const S: string; const Default: Double): Double;
var
  V: Double;
begin
  if TryStrToFloat(S, V) then
    StrToFloatDef := V
  else
    StrToFloatDef := Default;
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

function BrutoIsPathSep(C: Char): Boolean;
begin
  {$IFDEF WINDOWS}
  BrutoIsPathSep := (C = '/') or (C = '\') or (C = ':');
  {$ELSE}
  BrutoIsPathSep := C = '/';
  {$ENDIF}
end;

{ The index of FileName's extension's '.', or 0. }
function BrutoExtensionAt(const FileName: string): Integer;
var
  I: Integer;
begin
  I := Length(FileName);
  while (I > 0) and not BrutoIsPathSep(FileName[I]) and (FileName[I] <> '.') do
    Dec(I);
  if (I > 1) and (FileName[I] = '.') and not BrutoIsPathSep(FileName[I - 1]) then
    Result := I
  else
    Result := 0;
end;

function ChangeFileExt(const FileName, Extension: string): string;
var
  I: Integer;
begin
  I := BrutoExtensionAt(FileName);
  if I = 0 then
    I := Length(FileName) + 1;
  Result := Copy(FileName, 1, I - 1) + Extension;
end;

function ExtractFileExt(const FileName: string): string;
var
  I: Integer;
begin
  I := BrutoExtensionAt(FileName);
  if I = 0 then
    Result := ''
  else
    Result := Copy(FileName, I, MaxInt);
end;

{ The index of FileName's last separator, or 0. }
function BrutoLastSep(const FileName: string): Integer;
begin
  Result := Length(FileName);
  while (Result > 0) and not BrutoIsPathSep(FileName[Result]) do
    Dec(Result);
end;

function ExtractFileName(const FileName: string): string;
begin
  Result := Copy(FileName, BrutoLastSep(FileName) + 1, MaxInt);
end;

function ExtractFilePath(const FileName: string): string;
begin
  Result := Copy(FileName, 1, BrutoLastSep(FileName));
end;

function ExtractFileDir(const FileName: string): string;
var
  I: Integer;
begin
  I := BrutoLastSep(FileName);
  if (I > 1) and not BrutoIsPathSep(FileName[I - 1]) then
    Dec(I);
  Result := Copy(FileName, 1, I);
end;

function IncludeTrailingPathDelimiter(const Path: string): string;
begin
  Result := Path;
  if (Result = '') or not BrutoIsPathSep(Result[Length(Result)]) then
    Result := Result + {$IFDEF WINDOWS}'\'{$ELSE}'/'{$ENDIF};
end;

function ExcludeTrailingPathDelimiter(const Path: string): string;
begin
  Result := Path;
  if (Result <> '') and BrutoIsPathSep(Result[Length(Result)]) then
    SetLength(Result, Length(Result) - 1);
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

function CurrToStr(Value: Currency): string;
begin
  Result := FloatToStr(Value);
end;

function StrToCurr(const S: string): Currency;
begin
  Result := StrToFloat(S);
end;

function TryStrToCurr(const S: string; out Value: Currency): Boolean;
var
  D: Double;
begin
  Result := TryStrToFloat(S, D);
  if Result then
    Value := D;
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

constructor EVariantError.CreateCode(Code: LongInt);
begin
  case Code of
    LongInt($80020005): Create('Invalid variant type cast');
    LongInt($80020008): Create('Invalid variant type');
    LongInt($8002000A): Create('Variant overflow');
    LongInt($8002000B): Create('Variant array bounds error');
    LongInt($80004001): Create('Operation not supported');
    LongInt($80070057): Create('Invalid argument');
  else
    Create('Unexpected variant error');
  end;
  ErrCode := Code;
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

{ ── Dates and times (FPC's dati.inc) ─────────────────────────── }

type
  TBrutoTm = record
    tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year, tm_wday, tm_yday,
    tm_isdst: LongInt;
    { Unix: tm_gmtoff and tm_zone follow (room for them). }
    Rest: array[0..3] of Int64;
  end;

{$IFDEF WINDOWS}
type
  TBrutoWinSystemTime = record
    wYear, wMonth, wDayOfWeek, wDay, wHour, wMinute, wSecond,
    wMilliseconds: Word;
  end;

procedure BrutoWinGetLocalTime(var ST: TBrutoWinSystemTime); stdcall;
  external 'kernel32' name 'GetLocalTime';
function bruto_localtime64_s(Tm: Pointer; T: Pointer): LongInt; cdecl;
  external 'c' name '_localtime64_s';
function bruto_mktime64(Tm: Pointer): Int64; cdecl; external 'c' name '_mktime64';
function bruto_mkgmtime64(Tm: Pointer): Int64; cdecl; external 'c' name '_mkgmtime64';
function bruto_time64(T: Pointer): Int64; cdecl; external 'c' name '_time64';
{$ELSE}
type
  TBrutoTimeVal = record
    tv_sec: Int64;
    tv_usec: Int64;
  end;

function bruto_gettimeofday(Tv: Pointer; Tz: Pointer): LongInt; cdecl;
  external 'c' name 'gettimeofday';
function bruto_localtime_r(T: Pointer; Tm: Pointer): Pointer; cdecl;
  external 'c' name 'localtime_r';
function bruto_mktime(Tm: Pointer): Int64; cdecl; external 'c' name 'mktime';
{$ENDIF}

procedure BrutoLocalTm(T: Int64; var Tm: TBrutoTm);
begin
  FillChar(Tm, SizeOf(Tm), 0);
  {$IFDEF WINDOWS}
  bruto_localtime64_s(@Tm, @T);
  {$ELSE}
  bruto_localtime_r(@T, @Tm);
  {$ENDIF}
end;

procedure GetLocalTime(var SystemTime: TSystemTime);
var
{$IFDEF WINDOWS}
  ST: TBrutoWinSystemTime;
{$ELSE}
  Tv: TBrutoTimeVal;
  Tm: TBrutoTm;
{$ENDIF}
begin
  {$IFDEF WINDOWS}
  BrutoWinGetLocalTime(ST);
  SystemTime.Year := ST.wYear;
  SystemTime.Month := ST.wMonth;
  SystemTime.Day := ST.wDay;
  SystemTime.DayOfWeek := ST.wDayOfWeek;
  SystemTime.Hour := ST.wHour;
  SystemTime.Minute := ST.wMinute;
  SystemTime.Second := ST.wSecond;
  SystemTime.MilliSecond := ST.wMilliseconds;
  {$ELSE}
  bruto_gettimeofday(@Tv, nil);
  BrutoLocalTm(Tv.tv_sec, Tm);
  SystemTime.Year := Tm.tm_year + 1900;
  SystemTime.Month := Tm.tm_mon + 1;
  SystemTime.Day := Tm.tm_mday;
  SystemTime.DayOfWeek := Tm.tm_wday;
  SystemTime.Hour := Tm.tm_hour;
  SystemTime.Minute := Tm.tm_min;
  SystemTime.Second := Tm.tm_sec;
  SystemTime.MilliSecond := Tv.tv_usec div 1000;
  {$ENDIF}
end;

function DoEncodeDate(Year, Month, Day: Word): LongInt;
var
  D: TDateTime;
begin
  if TryEncodeDate(Year, Month, Day, D) then
    Result := Trunc(D)
  else
    Result := 0;
end;

function DoEncodeTime(Hour, Minute, Second, MilliSecond: Word): TDateTime;
begin
  if not TryEncodeTime(Hour, Minute, Second, MilliSecond, Result) then
    Result := 0;
end;

function ComposeDateTime(Date, Time: TDateTime): TDateTime;
begin
  if Date < 0 then
    Result := Trunc(Date) - Abs(Frac(Time))
  else
    Result := Trunc(Date) + Abs(Frac(Time));
end;

function DateTimeToTimeStamp(DateTime: TDateTime): TTimeStamp;
var
  D: Double;
begin
  if not ((DateTime >= MinDateTime) and (DateTime <= MaxDateTime)) then
    raise EConvertError.Create(Format('%f is not a valid date/time value.', [DateTime]));
  D := DateTime * MSecsPerDay;
  if D < 0 then
    D := D - 0.5
  else
    D := D + 0.5;
  Result.Time := Abs(Trunc(D)) mod MSecsPerDay;
  Result.Date := DateDelta + Trunc(D) div MSecsPerDay;
end;

function TimeStampToDateTime(const TimeStamp: TTimeStamp): TDateTime;
begin
  Result := ComposeDateTime(TimeStamp.Date - DateDelta, TimeStamp.Time / MSecsPerDay);
end;

function MSecsToTimeStamp(MSecs: Int64): TTimeStamp;
begin
  Result.Date := MSecs div MSecsPerDay;
  Result.Time := MSecs - Int64(Result.Date) * MSecsPerDay;
end;

function TimeStampToMSecs(const TimeStamp: TTimeStamp): Int64;
begin
  Result := TimeStamp.Time + Int64(TimeStamp.Date) * MSecsPerDay;
end;

function TryEncodeDate(Year, Month, Day: Word; out Date: TDateTime): Boolean;
var
  C, YA: Int64;
  M, Y: Int64;
begin
  Result := (Year > 0) and (Year < 10000) and (Month >= 1) and (Month <= 12)
    and (Day > 0) and (Day <= MonthDays[IsLeapYear(Year)][Month]);
  if Result then
  begin
    M := Month;
    Y := Year;
    if M > 2 then
      Dec(M, 3)
    else
    begin
      Inc(M, 9);
      Dec(Y);
    end;
    C := Y div 100;
    YA := Y - 100 * C;
    Date := (146097 * C) shr 2 + (1461 * YA) shr 2 + (153 * M + 2) div 5 + Day;
    Date := Date - 693900;
  end;
end;

function TryEncodeTime(Hour, Min, Sec, MSec: Word; out Time: TDateTime): Boolean;
begin
  Result := ((Hour < 24) and (Min < 60) and (Sec < 60) and (MSec < 1000))
    or ((Hour = 23) and (Min = 59) and (Sec = 60) and (MSec < 1000));
  if Result then
    Time := (Int64(Hour) * 3600000 + Int64(Min) * 60000 + Int64(Sec) * 1000 + MSec) / MSecsPerDay;
end;

function EncodeDate(Year, Month, Day: Word): TDateTime;
begin
  if not TryEncodeDate(Year, Month, Day, Result) then
    raise EConvertError.Create(Format('%d-%d-%d is not a valid date specification',
      [Year, Month, Day]));
end;

function EncodeTime(Hour, Minute, Second, MilliSecond: Word): TDateTime;
begin
  if not TryEncodeTime(Hour, Minute, Second, MilliSecond, Result) then
    raise EConvertError.Create(Format('%d:%d:%d.%d is not a valid time specification',
      [Hour, Minute, Second, MilliSecond]));
end;

procedure DecodeDate(Date: TDateTime; out Year, Month, Day: Word);
var
  LY, LD, LM, J: Int64;
begin
  if Date <= -DateDelta then
  begin
    Year := 0;
    Month := 0;
    Day := 0;
  end
  else
  begin
    if Date > 0 then
      Date := Date + 1 / (MSecsPerDay * 2)
    else
      Date := Date - 1 / (MSecsPerDay * 2);
    if Date > MaxDateTime then
      Date := MaxDateTime;
    J := ((Trunc(Int(Date)) + 693900) shl 2) - 1;
    LY := J div 146097;
    J := J - 146097 * LY;
    LD := J shr 2;
    J := (LD shl 2 + 3) div 1461;
    LD := (LD shl 2 + 7 - 1461 * J) shr 2;
    LM := (5 * LD - 3) div 153;
    LD := (5 * LD + 2 - 153 * LM) div 5;
    LY := 100 * LY + J;
    if LM < 10 then
      Inc(LM, 3)
    else
    begin
      Dec(LM, 9);
      Inc(LY);
    end;
    Year := LY;
    Month := LM;
    Day := LD;
  end;
end;

function DecodeDateFully(const DateTime: TDateTime; out Year, Month, Day, DOW: Word): Boolean;
begin
  DecodeDate(DateTime, Year, Month, Day);
  DOW := DayOfWeek(DateTime);
  Result := IsLeapYear(Year);
end;

procedure DecodeTime(Time: TDateTime; out Hour, Minute, Second, MilliSecond: Word);
var
  L: Int64;
begin
  L := DateTimeToTimeStamp(Time).Time;
  Hour := L div 3600000;
  L := L mod 3600000;
  Minute := L div 60000;
  L := L mod 60000;
  Second := L div 1000;
  MilliSecond := L mod 1000;
end;

procedure DateTimeToSystemTime(DateTime: TDateTime; out SystemTime: TSystemTime);
begin
  DecodeDateFully(DateTime, SystemTime.Year, SystemTime.Month, SystemTime.Day,
    SystemTime.DayOfWeek);
  DecodeTime(DateTime, SystemTime.Hour, SystemTime.Minute, SystemTime.Second,
    SystemTime.MilliSecond);
  Dec(SystemTime.DayOfWeek);
end;

function SystemTimeToDateTime(const SystemTime: TSystemTime): TDateTime;
begin
  Result := ComposeDateTime(DoEncodeDate(SystemTime.Year, SystemTime.Month, SystemTime.Day),
    DoEncodeTime(SystemTime.Hour, SystemTime.Minute, SystemTime.Second, SystemTime.MilliSecond));
end;

function DayOfWeek(DateTime: TDateTime): Integer;
begin
  Result := 1 + ((Trunc(DateTime) - 1) mod 7);
  if Result <= 0 then
    Inc(Result, 7);
end;

function Date: TDateTime;
var
  ST: TSystemTime;
begin
  GetLocalTime(ST);
  Result := DoEncodeDate(ST.Year, ST.Month, ST.Day);
end;

function Time: TDateTime;
var
  ST: TSystemTime;
begin
  GetLocalTime(ST);
  Result := DoEncodeTime(ST.Hour, ST.Minute, ST.Second, ST.MilliSecond);
end;

function Now: TDateTime;
var
  ST: TSystemTime;
begin
  GetLocalTime(ST);
  Result := SystemTimeToDateTime(ST);
end;

function CurrentYear: Word;
var
  ST: TSystemTime;
begin
  GetLocalTime(ST);
  Result := ST.Year;
end;

function IncMonth(const DateTime: TDateTime; NumberOfMonths: Integer = 1): TDateTime;
var
  Year, Month, Day: Word;
begin
  DecodeDate(DateTime, Year, Month, Day);
  IncAMonth(Year, Month, Day, NumberOfMonths);
  Result := ComposeDateTime(DoEncodeDate(Year, Month, Day), DateTime);
end;

procedure IncAMonth(var Year, Month, Day: Word; NumberOfMonths: Integer = 1);
var
  TempMonth, S: Integer;
begin
  if NumberOfMonths >= 0 then
    S := 1
  else
    S := -1;
  Year := Year + NumberOfMonths div 12;
  TempMonth := Month + (NumberOfMonths mod 12) - 1;
  if (TempMonth > 11) or (TempMonth < 0) then
  begin
    Dec(TempMonth, S * 12);
    Year := Year + S;
  end;
  Month := TempMonth + 1;
  if Day > MonthDays[IsLeapYear(Year)][Month] then
    Day := MonthDays[IsLeapYear(Year)][Month];
end;

function IsLeapYear(Year: Word): Boolean;
begin
  if Year mod 100 <> 0 then
    Result := (Year mod 4) = 0
  else
    Result := (Year mod 16) = 0;
end;

function DateToStr(Date: TDateTime): string;
begin
  DateTimeToString(Result, 'ddddd', Date);
end;

function TimeToStr(Time: TDateTime): string;
begin
  DateTimeToString(Result, 'tt', Time);
end;

function DateTimeToStr(DateTime: TDateTime; ForceTimeIfZero: Boolean = False): string;
begin
  if ForceTimeIfZero then
    DateTimeToString(Result, 'f', DateTime)
  else
    DateTimeToString(Result, 'c', DateTime);
end;

{ The date in S, read in the order UseFormat gives day, month and year;
  ErrorMsg says why it isn't one. }
function BrutoStrToDate(out ErrorMsg: string; const S, UseFormat: string;
  Separator: Char): TDateTime;
var
  DF, S1: string;
  D, M, Y, LY, Len, N, I, C: Integer;
  DP, MP, YP, Which: Integer;
  Values: array[0..3] of LongInt;
  LocalTime: TSystemTime;
  YearMoreThanTwoDigits: Boolean;
begin
  ErrorMsg := '';
  Result := 0;
  Len := Length(S);
  while (Len > 0) and (S[Len] in [' ', #8, #9, #10, #12, #13]) do
    Dec(Len);
  if Len = 0 then
  begin
    ErrorMsg := '"" is not a valid date format';
    Exit;
  end;
  YearMoreThanTwoDigits := False;
  if Separator = #0 then
    Separator := DefaultFormatSettings.DateSeparator;
  DF := UpperCase(UseFormat);
  YP := 0;
  MP := 0;
  DP := 0;
  Which := 0;
  I := 0;
  while (I < Length(DF)) and (Which < 3) do
  begin
    Inc(I);
    case DF[I] of
      'Y': if YP = 0 then begin Inc(Which); YP := Which; end;
      'M': if MP = 0 then begin Inc(Which); MP := Which; end;
      'D': if DP = 0 then begin Inc(Which); DP := Which; end;
    end;
  end;
  for I := 0 to 3 do
    Values[I] := 0;
  S1 := '';
  N := 0;
  for I := 1 to Len do
  begin
    if S[I] in ['0'..'9'] then
      S1 := S1 + S[I];
    if (Separator <> ' ') and (S[I] = ' ') then
      Continue;
    if (S[I] = Separator) or ((I = Len) and (S[I] in ['0'..'9'])) then
    begin
      Inc(N);
      if N > 3 then
      begin
        ErrorMsg := '"' + S + '" is not a valid date format';
        Exit;
      end;
      if (N = YP) and (Length(S1) > 2) then
        YearMoreThanTwoDigits := True;
      Val(S1, Values[N], C);
      if C <> 0 then
      begin
        ErrorMsg := '"' + S + '" is not a valid date format';
        Exit;
      end;
      S1 := '';
    end
    else if not (S[I] in ['0'..'9']) then
    begin
      ErrorMsg := '"' + S + '" is not a valid date format';
      Exit;
    end;
  end;
  if (Which < 3) and (N > Which) then
  begin
    ErrorMsg := '"' + S + '" is not a valid date format';
    Exit;
  end;
  GetLocalTime(LocalTime);
  LY := LocalTime.Year;
  for I := 1 to 3 do
    if Values[I] > High(Word) then
    begin
      ErrorMsg := 'Invalid date';
      Exit;
    end;
  if N = 3 then
  begin
    Y := Values[YP];
    M := Values[MP];
    D := Values[DP];
  end
  else
  begin
    Y := LY;
    if N < 2 then
    begin
      D := Values[1];
      M := LocalTime.Month;
    end
    else if DP < MP then
    begin
      D := Values[1];
      M := Values[2];
    end
    else
    begin
      D := Values[2];
      M := Values[1];
    end;
  end;
  if (Y >= 0) and (Y < 100) and not YearMoreThanTwoDigits then
  begin
    LY := LY - DefaultFormatSettings.TwoDigitYearCenturyWindow;
    Inc(Y, LY div 100 * 100);
    if (DefaultFormatSettings.TwoDigitYearCenturyWindow > 0) and (Y < LY) then
      Inc(Y, 100);
  end;
  if not TryEncodeDate(Y, M, D, Result) then
    ErrorMsg := 'Invalid date';
end;

function StrToDate(const S: string; const UseFormat: string; Separator: Char): TDateTime;
var
  Msg: string;
begin
  Result := BrutoStrToDate(Msg, S, UseFormat, Separator);
  if Msg <> '' then
    raise EConvertError.Create(Msg);
end;

function StrToDate(const S: string; Separator: Char): TDateTime;
begin
  Result := StrToDate(S, DefaultFormatSettings.ShortDateFormat, Separator);
end;

function StrToDate(const S: string): TDateTime;
begin
  Result := StrToDate(S, DefaultFormatSettings.ShortDateFormat, #0);
end;

{ The time in S; ErrorMsg says why it isn't one. }
function BrutoStrToTime(out ErrorMsg: string; const S: string; Separator: Char): TDateTime;
const
  AmPmNone = 0;
  AmPmAM = 1;
  AmPmPM = 2;
var
  AmPm: Integer;
  TimeValues: array[0..3] of Word;

  function SplitElements: Boolean;
  var
    Cur, Offset, ElemLen, Err, TimeIndex, FirstSignificantDigit, Len: Integer;
    Value: Word;
    DigitPending, MSecPending: Boolean;
    AmPmStr: string;
    CurChar: Char;
  begin
    Result := False;
    AmPm := AmPmNone;
    MSecPending := False;
    TimeIndex := 0;
    for Cur := 0 to 3 do
      TimeValues[Cur] := 0;
    Len := Length(S);
    Cur := 1;
    while (Cur <= Len) and (S[Cur] = ' ') do
      Inc(Cur);
    if (Cur > Len) or (S[Cur] = Separator) or (S[Cur] = DefaultFormatSettings.DecimalSeparator) then
      Exit;
    DigitPending := S[Cur] in ['0'..'9'];
    while Cur <= Len do
    begin
      CurChar := S[Cur];
      if CurChar in ['0'..'9'] then
      begin
        if (not DigitPending) or (TimeIndex > 3) then
          Exit;
        Offset := Cur;
        if CurChar <> '0' then
          FirstSignificantDigit := Offset
        else
          FirstSignificantDigit := -1;
        while (Cur < Len) and (S[Cur + 1] in ['0'..'9']) do
        begin
          if (FirstSignificantDigit = -1) and (S[Cur] <> '0') then
            FirstSignificantDigit := Cur;
          Inc(Cur);
        end;
        if FirstSignificantDigit = -1 then
          FirstSignificantDigit := Cur;
        ElemLen := 1 + Cur - FirstSignificantDigit;
        if (ElemLen <= 2) or ((ElemLen <= 3) and (TimeIndex = 3)) then
        begin
          Val(Copy(S, FirstSignificantDigit, ElemLen), Value, Err);
          TimeValues[TimeIndex] := Value;
          Inc(TimeIndex);
          DigitPending := False;
        end
        else
          Exit;
      end
      else if CurChar = ' ' then
      begin
      end
      else if CurChar = Separator then
      begin
        if DigitPending or (TimeIndex > 2) then
          Exit;
        DigitPending := True;
        MSecPending := False;
      end
      else if CurChar = DefaultFormatSettings.DecimalSeparator then
      begin
        if DigitPending or MSecPending or (TimeIndex <> 3) then
          Exit;
        DigitPending := True;
        MSecPending := True;
      end
      else
      begin
        if (AmPm <> AmPmNone) or DigitPending then
          Exit;
        Offset := Cur;
        while (Cur < Len) and not (S[Cur + 1] in [Separator, ' ',
          DefaultFormatSettings.DecimalSeparator]) and not (S[Cur + 1] in ['0'..'9']) do
          Inc(Cur);
        ElemLen := 1 + Cur - Offset;
        AmPmStr := Copy(S, Offset, ElemLen);
        if CompareText(AmPmStr, DefaultFormatSettings.TimeAMString) = 0 then
          AmPm := AmPmAM
        else if CompareText(AmPmStr, DefaultFormatSettings.TimePMString) = 0 then
          AmPm := AmPmPM
        else if CompareText(AmPmStr, 'AM') = 0 then
          AmPm := AmPmAM
        else if CompareText(AmPmStr, 'PM') = 0 then
          AmPm := AmPmPM
        else
          Exit;
        if TimeIndex = 0 then
          DigitPending := True
        else
        begin
          TimeIndex := 4;
          DigitPending := False;
        end;
      end;
      Inc(Cur);
    end;
    if (TimeIndex = 0) or ((AmPm <> AmPmNone) and ((TimeValues[0] > 12)
      or (TimeValues[0] = 0))) or DigitPending then
      Exit;
    Result := True;
  end;

begin
  ErrorMsg := '';
  Result := 0;
  if Separator = #0 then
    Separator := DefaultFormatSettings.TimeSeparator;
  AmPm := AmPmNone;
  if not SplitElements then
  begin
    ErrorMsg := '"' + S + '" is not a valid time format';
    Exit;
  end;
  if (AmPm = AmPmPM) and (TimeValues[0] <> 12) then
    Inc(TimeValues[0], 12)
  else if (AmPm = AmPmAM) and (TimeValues[0] = 12) then
    TimeValues[0] := 0;
  if not TryEncodeTime(TimeValues[0], TimeValues[1], TimeValues[2], TimeValues[3], Result) then
    ErrorMsg := '"' + S + '" is not a valid time format';
end;

function StrToTime(const S: string; Separator: Char): TDateTime;
var
  Msg: string;
begin
  Result := BrutoStrToTime(Msg, S, Separator);
  if Msg <> '' then
    raise EConvertError.Create(Msg);
end;

function StrToTime(const S: string): TDateTime;
begin
  Result := StrToTime(S, #0);
end;

{ S split into its date and its time: how many parts there are. }
function BrutoSplitDateTime(const DateTimeStr: string; out DateStr, TimeStr: string): Integer;
var
  P: Integer;
  S, Msg: string;
begin
  Result := 0;
  DateStr := '';
  TimeStr := '';
  S := Trim(DateTimeStr);
  if S = '' then
    Exit;
  P := 1;
  while (P < Length(S)) and not (S[P + 1] in [#9, #10, #13, ' ']) do
    Inc(P);
  DateStr := Copy(S, 1, P);
  TimeStr := Trim(Copy(S, P + 1, MaxInt));
  if TimeStr <> '' then
    Result := 2
  else
  begin
    Result := 1;
    if ((DefaultFormatSettings.DateSeparator <> DefaultFormatSettings.TimeSeparator)
      and (Pos(DefaultFormatSettings.TimeSeparator, DateStr) > 0))
      or ((DefaultFormatSettings.DateSeparator = DefaultFormatSettings.TimeSeparator)
      and (BrutoStrToDate(Msg, DateStr, DefaultFormatSettings.ShortDateFormat, #0) = 0)
      and (Msg <> '')) then
    begin
      TimeStr := DateStr;
      DateStr := '';
    end;
  end;
end;

function StrToDateTime(const S: string): TDateTime;
var
  DateStr, TimeStr: string;
begin
  case BrutoSplitDateTime(S, DateStr, TimeStr) of
    0: Result := StrToDate('');
    1:
      if DateStr <> '' then
        Result := StrToDate(DateStr)
      else
        Result := StrToTime(TimeStr);
  else
    Result := ComposeDateTime(StrToDate(DateStr), StrToTime(TimeStr));
  end;
end;

function TryStrToDate(const S: string; out Value: TDateTime): Boolean;
var
  Msg: string;
begin
  Result := Length(S) <> 0;
  if Result then
  begin
    Value := BrutoStrToDate(Msg, S, DefaultFormatSettings.ShortDateFormat, #0);
    Result := Msg = '';
  end;
end;

function TryStrToTime(const S: string; out Value: TDateTime): Boolean;
var
  Msg: string;
begin
  Result := Length(S) <> 0;
  if Result then
  begin
    Value := BrutoStrToTime(Msg, S, #0);
    Result := Msg = '';
  end;
end;

function TryStrToDateTime(const S: string; out Value: TDateTime): Boolean;
var
  DateStr, TimeStr: string;
  D, T: TDateTime;
begin
  Result := False;
  case BrutoSplitDateTime(S, DateStr, TimeStr) of
    1:
      if DateStr <> '' then
        Result := TryStrToDate(DateStr, Value)
      else
        Result := TryStrToTime(TimeStr, Value);
    2:
      begin
        Result := TryStrToDate(DateStr, D) and TryStrToTime(TimeStr, T);
        if Result then
          Value := ComposeDateTime(D, T);
      end;
  end;
end;

function StrToDateDef(const S: string; const DefValue: TDateTime): TDateTime;
begin
  if not TryStrToDate(S, Result) then
    Result := DefValue;
end;

function StrToTimeDef(const S: string; const DefValue: TDateTime): TDateTime;
begin
  if not TryStrToTime(S, Result) then
    Result := DefValue;
end;

function StrToDateTimeDef(const S: string; const DefValue: TDateTime): TDateTime;
begin
  if not TryStrToDateTime(S, Result) then
    Result := DefValue;
end;

function FormatDateTime(const FormatStr: string; DateTime: TDateTime;
  Options: TFormatDateTimeOptions = []): string;
begin
  DateTimeToString(Result, FormatStr, DateTime, Options);
end;

{ Whether S has Sub (ignoring case) at index I. }
function BrutoMatchAt(const S: string; I: Integer; const Sub: string): Boolean;
begin
  Result := CompareText(Copy(S, I, Length(Sub)), Sub) = 0;
end;

procedure DateTimeToString(out Result: string; const FormatStr: string;
  const DateTime: TDateTime; Options: TFormatDateTimeOptions = []);
var
  Year, Month, Day, DayOfWeek, Hour, Minute, Second, MilliSecond: Word;
  Output: string;

  procedure StoreInt(Value, Digits: Integer);
  var
    S: string;
  begin
    S := IntToStr(Value);
    while Length(S) < Digits do
      S := '0' + S;
    Output := Output + S;
  end;

  function FullDays(ADateTime: TDateTime): Integer;
  begin
    if ADateTime < 0 then
      ADateTime := -ADateTime;
    Result := Trunc(ADateTime);
    if (Frac(ADateTime) > 0.9) and (Hour = 0) and (Minute = 0) and (Second = 0)
      and (MilliSecond = 0) then
      Inc(Result);
  end;

  procedure StoreFormat(const Fmt: string; Nesting: Integer; TimeFlag: Boolean);
  var
    Token, LastFormatToken, PrevLastToken: Char;
    Cur, P, Count, Tmp: Integer;
    Clock12, IsInterval: Boolean;
  begin
    if Nesting > 1 then
      Exit;
    Clock12 := False;
    IsInterval := False;
    { An unquoted 12-hour clock token makes h a 12-hour hour. }
    P := 1;
    while P <= Length(Fmt) do
    begin
      Token := Fmt[P];
      if (Token = '''') or (Token = '"') then
      begin
        Inc(P);
        while (P <= Length(Fmt)) and (Fmt[P] <> Token) do
          Inc(P);
      end
      else if (Token = 'A') or (Token = 'a') then
        if BrutoMatchAt(Fmt, P, 'A/P') or BrutoMatchAt(Fmt, P, 'AMPM')
          or BrutoMatchAt(Fmt, P, 'AM/PM') then
        begin
          Clock12 := True;
          Break;
        end;
      Inc(P);
    end;
    LastFormatToken := ' ';
    PrevLastToken := 'H';
    Cur := 1;
    while Cur <= Length(Fmt) do
    begin
      Token := UpCase(Fmt[Cur]);
      Count := 1;
      P := Cur + 1;
      case Token of
        '''', '"':
          begin
            while (P <= Length(Fmt)) and (Fmt[P] <> Fmt[Cur]) do
              Inc(P);
            Inc(P);
            Count := P - Cur;
            Output := Output + Copy(Fmt, Cur + 1, Count - 2);
          end;
        'A':
          if BrutoMatchAt(Fmt, Cur, 'AMPM') then
          begin
            Count := 4;
            if Hour < 12 then
              Output := Output + DefaultFormatSettings.TimeAMString
            else
              Output := Output + DefaultFormatSettings.TimePMString;
          end
          else if BrutoMatchAt(Fmt, Cur, 'AM/PM') then
          begin
            Count := 5;
            if Hour < 12 then
              Output := Output + Copy(Fmt, Cur, 2)
            else
              Output := Output + Copy(Fmt, Cur + 3, 2);
          end
          else if BrutoMatchAt(Fmt, Cur, 'A/P') then
          begin
            Count := 3;
            if Hour < 12 then
              Output := Output + Fmt[Cur]
            else
              Output := Output + Fmt[Cur + 2];
          end
          else
            raise EConvertError.Create('Illegal character in format string');
        '/':
          if DefaultFormatSettings.DateSeparator <> #0 then
            Output := Output + DefaultFormatSettings.DateSeparator;
        ':':
          if DefaultFormatSettings.TimeSeparator <> #0 then
            Output := Output + DefaultFormatSettings.TimeSeparator;
        '[':
          if fdoInterval in Options then
            IsInterval := True
          else
            Output := Output + '[';
        ']':
          if fdoInterval in Options then
            IsInterval := False
          else
            Output := Output + ']';
        ' ', 'C', 'D', 'H', 'M', 'N', 'S', 'T', 'Y', 'Z', 'F':
          begin
            while (P <= Length(Fmt)) and (UpCase(Fmt[P]) = Token) do
              Inc(P);
            Count := P - Cur;
            case Token of
              ' ': Output := Output + Copy(Fmt, Cur, Count);
              'Y':
                if Count > 2 then
                  StoreInt(Year, 4)
                else
                  StoreInt(Year mod 100, 2);
              'M':
                if IsInterval and ((PrevLastToken = 'H') or TimeFlag) then
                  StoreInt(Minute + (Hour + FullDays(DateTime) * 24) * 60, 0)
                else if (LastFormatToken = 'H') or TimeFlag then
                begin
                  if Count = 1 then
                    StoreInt(Minute, 0)
                  else
                    StoreInt(Minute, 2);
                end
                else
                  case Count of
                    1: StoreInt(Month, 0);
                    2: StoreInt(Month, 2);
                    3: Output := Output + DefaultFormatSettings.ShortMonthNames[Month];
                  else
                    Output := Output + DefaultFormatSettings.LongMonthNames[Month];
                  end;
              'D':
                case Count of
                  1: StoreInt(Day, 0);
                  2: StoreInt(Day, 2);
                  3: Output := Output + DefaultFormatSettings.ShortDayNames[DayOfWeek];
                  4: Output := Output + DefaultFormatSettings.LongDayNames[DayOfWeek];
                  5: StoreFormat(DefaultFormatSettings.ShortDateFormat, Nesting + 1, False);
                else
                  StoreFormat(DefaultFormatSettings.LongDateFormat, Nesting + 1, False);
                end;
              'H':
                if IsInterval then
                  StoreInt(Hour + FullDays(DateTime) * 24, Count)
                else if Clock12 then
                begin
                  Tmp := Hour mod 12;
                  if Tmp = 0 then
                    Tmp := 12;
                  if Count = 1 then
                    StoreInt(Tmp, 0)
                  else
                    StoreInt(Tmp, 2);
                end
                else if Count = 1 then
                  StoreInt(Hour, 0)
                else
                  StoreInt(Hour, 2);
              'N':
                if IsInterval then
                  StoreInt(Minute + (Hour + FullDays(DateTime) * 24) * 60, Count)
                else if Count = 1 then
                  StoreInt(Minute, 0)
                else
                  StoreInt(Minute, 2);
              'S':
                if IsInterval then
                  StoreInt(Second + (Minute + (Hour + FullDays(DateTime) * 24) * 60) * 60, Count)
                else if Count = 1 then
                  StoreInt(Second, 0)
                else
                  StoreInt(Second, 2);
              'Z':
                if Count = 1 then
                  StoreInt(MilliSecond, 0)
                else
                  StoreInt(MilliSecond, 3);
              'T':
                if Count = 1 then
                  StoreFormat(DefaultFormatSettings.ShortTimeFormat, Nesting + 1, True)
                else
                  StoreFormat(DefaultFormatSettings.LongTimeFormat, Nesting + 1, True);
              'C':
                begin
                  StoreFormat(DefaultFormatSettings.ShortDateFormat, Nesting + 1, False);
                  if (Hour <> 0) or (Minute <> 0) or (Second <> 0) then
                  begin
                    Output := Output + ' ';
                    StoreFormat(DefaultFormatSettings.LongTimeFormat, Nesting + 1, True);
                  end;
                end;
              'F':
                begin
                  StoreFormat(DefaultFormatSettings.ShortDateFormat, Nesting + 1, False);
                  Output := Output + ' ';
                  StoreFormat(DefaultFormatSettings.LongTimeFormat, Nesting + 1, True);
                end;
            end;
            PrevLastToken := LastFormatToken;
            LastFormatToken := Token;
          end;
      else
        Output := Output + Fmt[Cur];
      end;
      Inc(Cur, Count);
    end;
  end;

begin
  DecodeDateFully(DateTime, Year, Month, Day, DayOfWeek);
  DecodeTime(DateTime, Hour, Minute, Second, MilliSecond);
  Output := '';
  if FormatStr <> '' then
    StoreFormat(FormatStr, 0, False)
  else
    StoreFormat('C', 0, False);
  if (DateTime < 0) and (fdoInterval in Options) then
    Output := '-' + Output;
  Result := Output;
end;

procedure ReplaceTime(var DateTime: TDateTime; NewTime: TDateTime);
begin
  DateTime := ComposeDateTime(DateTime, NewTime);
end;

procedure ReplaceDate(var DateTime: TDateTime; const NewDate: TDateTime);
var
  Tmp: TDateTime;
begin
  Tmp := NewDate;
  ReplaceTime(Tmp, DateTime);
  DateTime := Tmp;
end;

function FileDateToDateTime(FileDate: Int64): TDateTime;
var
  Tm: TBrutoTm;
begin
  BrutoLocalTm(FileDate, Tm);
  Result := ComposeDateTime(DoEncodeDate(Tm.tm_year + 1900, Tm.tm_mon + 1, Tm.tm_mday),
    DoEncodeTime(Tm.tm_hour, Tm.tm_min, Tm.tm_sec, 0));
end;

function DateTimeToFileDate(DateTime: TDateTime): Int64;
var
  Tm: TBrutoTm;
  YY, MM, DD, H, M, S, MS: Word;
begin
  DecodeDate(DateTime, YY, MM, DD);
  DecodeTime(DateTime, H, M, S, MS);
  FillChar(Tm, SizeOf(Tm), 0);
  Tm.tm_year := YY - 1900;
  Tm.tm_mon := MM - 1;
  Tm.tm_mday := DD;
  Tm.tm_hour := H;
  Tm.tm_min := M;
  Tm.tm_sec := S;
  Tm.tm_isdst := -1;
  {$IFDEF WINDOWS}
  Result := bruto_mktime64(@Tm);
  {$ELSE}
  Result := bruto_mktime(@Tm);
  {$ENDIF}
end;

function GetLocalTimeOffset: Integer;
var
  Tm: TBrutoTm;
{$IFDEF WINDOWS}
  T: Int64;
begin
  T := bruto_time64(nil);
  BrutoLocalTm(T, Tm);
  Result := -((bruto_mkgmtime64(@Tm) - T) div 60);
end;
{$ELSE}
  Tv: TBrutoTimeVal;
begin
  bruto_gettimeofday(@Tv, nil);
  BrutoLocalTm(Tv.tv_sec, Tm);
  { tm_gmtoff: seconds east of UTC. }
  Result := -(Tm.Rest[0] div 60);
end;
{$ENDIF}

procedure BrutoInitFormatSettings;
const
  ShortMonths: array[1..12] of string = ('Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
    'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec');
  LongMonths: array[1..12] of string = ('January', 'February', 'March', 'April',
    'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December');
  ShortDays: array[1..7] of string = ('Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat');
  LongDays: array[1..7] of string = ('Sunday', 'Monday', 'Tuesday', 'Wednesday',
    'Thursday', 'Friday', 'Saturday');
var
  I: Integer;
begin
  with DefaultFormatSettings do
  begin
    CurrencyFormat := 1;
    NegCurrFormat := 5;
    ThousandSeparator := ',';
    DecimalSeparator := '.';
    CurrencyDecimals := 2;
    DateSeparator := '-';
    TimeSeparator := ':';
    ListSeparator := ',';
    CurrencyString := '$';
    ShortDateFormat := 'd/m/y';
    LongDateFormat := 'dd" "mmmm" "yyyy';
    TimeAMString := 'AM';
    TimePMString := 'PM';
    ShortTimeFormat := 'hh:nn';
    LongTimeFormat := 'hh:nn:ss';
    for I := 1 to 12 do
    begin
      ShortMonthNames[I] := ShortMonths[I];
      LongMonthNames[I] := LongMonths[I];
    end;
    for I := 1 to 7 do
    begin
      ShortDayNames[I] := ShortDays[I];
      LongDayNames[I] := LongDays[I];
    end;
    TwoDigitYearCenturyWindow := 50;
  end;
end;

{ ── Null-terminated strings ─────────────────────────────────── }

function StrLen(P: PChar): SizeInt;
begin
  Result := 0;
  if P <> nil then
    while P[Result] <> #0 do
      Inc(Result);
end;

function StrEnd(P: PChar): PChar;
begin
  Result := P + StrLen(P);
end;

function StrMove(Dest, Source: PChar; L: SizeInt): PChar;
begin
  Move(Source^, Dest^, L);
  Result := Dest;
end;

function StrCopy(Dest, Source: PChar): PChar;
begin
  Move(Source^, Dest^, StrLen(Source) + 1);
  Result := Dest;
end;

function StrLCopy(Dest, Source: PChar; MaxLen: SizeInt): PChar;
var
  I: SizeInt;
begin
  I := 0;
  while (I < MaxLen) and (Source[I] <> #0) do
  begin
    Dest[I] := Source[I];
    Inc(I);
  end;
  Dest[I] := #0;
  Result := Dest;
end;

function StrECopy(Dest, Source: PChar): PChar;
begin
  StrCopy(Dest, Source);
  Result := Dest + StrLen(Source);
end;

function StrPCopy(Dest: PChar; const Source: string): PChar;
begin
  if Length(Source) > 0 then
    Move(Source[1], Dest^, Length(Source));
  Dest[Length(Source)] := #0;
  Result := Dest;
end;

function StrPLCopy(Dest: PChar; const Source: string; MaxLen: SizeInt): PChar;
var
  L: SizeInt;
begin
  L := Length(Source);
  if L > MaxLen then
    L := MaxLen;
  if L > 0 then
    Move(Source[1], Dest^, L);
  Dest[L] := #0;
  Result := Dest;
end;

function StrCat(Dest, Source: PChar): PChar;
begin
  StrCopy(StrEnd(Dest), Source);
  Result := Dest;
end;

function StrLCat(Dest, Source: PChar; L: SizeInt): PChar;
var
  D: SizeInt;
begin
  D := StrLen(Dest);
  if D < L then
    StrLCopy(Dest + D, Source, L - D);
  Result := Dest;
end;

function BrutoUpChar(C: Char): Char;
begin
  if (C >= 'a') and (C <= 'z') then
    Result := Chr(Ord(C) - 32)
  else
    Result := C;
end;

function BrutoLowChar(C: Char): Char;
begin
  if (C >= 'A') and (C <= 'Z') then
    Result := Chr(Ord(C) + 32)
  else
    Result := C;
end;

{ Compare at most L characters (L < 0: all), ignoring case or not. }
function BrutoStrCompare(Str1, Str2: PChar; L: SizeInt; IgnoreCase: Boolean): SizeInt;
var
  I: SizeInt;
  C1, C2: Char;
begin
  Result := 0;
  I := 0;
  while (L < 0) or (I < L) do
  begin
    C1 := Str1[I];
    C2 := Str2[I];
    if IgnoreCase then
    begin
      C1 := BrutoUpChar(C1);
      C2 := BrutoUpChar(C2);
    end;
    Result := Ord(C1) - Ord(C2);
    if (Result <> 0) or (C1 = #0) then
      Exit;
    Inc(I);
  end;
end;

function StrComp(Str1, Str2: PChar): SizeInt;
begin
  Result := BrutoStrCompare(Str1, Str2, -1, False);
end;

function StrIComp(Str1, Str2: PChar): SizeInt;
begin
  Result := BrutoStrCompare(Str1, Str2, -1, True);
end;

function StrLComp(Str1, Str2: PChar; L: SizeInt): SizeInt;
begin
  Result := BrutoStrCompare(Str1, Str2, L, False);
end;

function StrLIComp(Str1, Str2: PChar; L: SizeInt): SizeInt;
begin
  Result := BrutoStrCompare(Str1, Str2, L, True);
end;

function StrScan(P: PChar; C: Char): PChar;
var
  I: SizeInt;
begin
  I := 0;
  while True do
  begin
    if P[I] = C then
      Exit(P + I);
    if P[I] = #0 then
      Exit(nil);
    Inc(I);
  end;
end;

function StrRScan(P: PChar; C: Char): PChar;
var
  I: SizeInt;
begin
  I := StrLen(P);
  while I >= 0 do
  begin
    if P[I] = C then
      Exit(P + I);
    Dec(I);
  end;
  Result := nil;
end;

function StrPos(Str1, Str2: PChar): PChar;
var
  I, L: SizeInt;
begin
  Result := nil;
  if (Str1 = nil) or (Str2 = nil) then
    Exit;
  L := StrLen(Str2);
  I := 0;
  while Str1[I] <> #0 do
  begin
    if StrLComp(Str1 + I, Str2, L) = 0 then
      Exit(Str1 + I);
    Inc(I);
  end;
  if L = 0 then
    Result := Str1 + I;
end;

function StrUpper(P: PChar): PChar;
var
  I: SizeInt;
begin
  I := 0;
  while P[I] <> #0 do
  begin
    P[I] := BrutoUpChar(P[I]);
    Inc(I);
  end;
  Result := P;
end;

function StrLower(P: PChar): PChar;
var
  I: SizeInt;
begin
  I := 0;
  while P[I] <> #0 do
  begin
    P[I] := BrutoLowChar(P[I]);
    Inc(I);
  end;
  Result := P;
end;

function StrPas(P: PChar): string;
var
  L: SizeInt;
begin
  L := StrLen(P);
  SetLength(Result, L);
  if L > 0 then
    Move(P^, Result[1], L);
end;

{ As FPC's: the size is kept in the 4 bytes before the string. }
function StrAlloc(Size: SizeInt): PChar;
var
  P: PChar;
begin
  Inc(Size, SizeOf(LongWord));
  GetMem(P, Size);
  PLongWord(P)^ := Size;
  Result := P + SizeOf(LongWord);
  Result^ := #0;
end;

function StrNew(P: PChar): PChar;
var
  L: SizeInt;
begin
  Result := nil;
  if (P = nil) or (P^ = #0) then
    Exit;
  L := StrLen(P) + 1;
  Result := StrAlloc(L);
  Move(P^, Result^, L);
end;

procedure StrDispose(P: PChar);
begin
  if P <> nil then
    FreeMem(P - SizeOf(LongWord));
end;

function AnsiStrComp(S1, S2: PChar): SizeInt;
begin
  Result := StrComp(S1, S2);
end;

function AnsiStrIComp(S1, S2: PChar): SizeInt;
begin
  Result := StrIComp(S1, S2);
end;

function AnsiStrLComp(S1, S2: PChar; MaxLen: SizeInt): SizeInt;
begin
  Result := StrLComp(S1, S2, MaxLen);
end;

function AnsiStrLIComp(S1, S2: PChar; MaxLen: SizeInt): SizeInt;
begin
  Result := StrLIComp(S1, S2, MaxLen);
end;

function CharInSet(C: AnsiChar; const CheckSet: TSysCharSet): Boolean;
begin
  Result := C in CheckSet;
end;

function CharInSet(C: WideChar; const CheckSet: TSysCharSet): Boolean;
begin
  Result := (Ord(C) < 256) and (AnsiChar(Ord(C)) in CheckSet);
end;

initialization
  BrutoRunErrorHook := @BrutoRunErrorToException;
  BrutoInitFormatSettings;
end.
