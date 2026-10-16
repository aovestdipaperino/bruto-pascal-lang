{ Bruto Pascal runtime: parts of FPC's System unit that are easiest to
  write in Pascal. Each declaration is used only when the program and its
  units don't declare one of the same name. }
unit BrutoSystem;

interface

type
  TFPCHeapStatus = record
    MaxHeapSize, MaxHeapUsed, CurrHeapSize, CurrHeapUsed, CurrHeapFree: PtrUInt;
  end;

  TSystemCodePage = Word;

  { Run-time type information, as `TypeInfo(T)` and a class's ClassInfo
    give it (the TypInfo unit reads it). FPC's names; the layout is
    Bruto's: a TTypeInfo, then its TTypeData. }
  TTypeKind = (tkUnknown, tkInteger, tkChar, tkEnumeration, tkFloat, tkSet,
    tkMethod, tkSString, tkLString, tkAString, tkWString, tkVariant, tkArray,
    tkRecord, tkInterface, tkClass, tkObject, tkWChar, tkBool, tkInt64,
    tkQWord, tkDynArray, tkInterfaceRaw, tkProcVar, tkUString, tkUChar,
    tkHelper, tkFile, tkClassRef, tkPointer);
  TOrdType = (otSByte, otUByte, otSWord, otUWord, otSLong, otULong, otSQWord,
    otUQWord);
  TFloatType = (ftSingle, ftDouble, ftExtended, ftComp, ftCurr);

  { A Variant's layout, as FPC's: its type, three reserved words and the
    value. A variant array's data is a dynamic array of variants, its low
    bound in Reserved1. }
  TVarType = Word;
  PVariant = ^Variant;
  PVarData = ^TVarData;
  TVarData = packed record
    VType: TVarType;
    Reserved1, Reserved2, Reserved3: Word;
    case Integer of
      0: (VSmallInt: SmallInt);
      1: (VInteger: LongInt);
      2: (VSingle: Single);
      3: (VDouble: Double);
      4: (VCurrency: Currency);
      5: (VDate: Double);
      6: (VOleStr: Pointer);
      7: (VBoolean: Boolean);
      8: (VShortInt: ShortInt);
      9: (VByte: Byte);
      10: (VWord: Word);
      11: (VLongWord: LongWord);
      12: (VInt64: Int64);
      13: (VQWord: QWord);
      14: (VString: Pointer);
      15: (VArray: Pointer);
      16: (VPointer: Pointer);
  end;

  PTypeInfo = ^TTypeInfo;
  PPTypeInfo = ^PTypeInfo;
  TTypeInfo = record
    Kind: TTypeKind;
    Name: ShortString;
  end;

  PTypeData = ^TTypeData;
  TTypeData = record
    { Ordinal types: their storage, bounds, and (an enumeration's) names,
      indexed by value - MinValue. }
    OrdType: TOrdType;
    MinValue, MaxValue: LongInt;
    BaseType: PTypeInfo;
    NameList: Pointer;
    { A set's element type. }
    CompType: PTypeInfo;
    MinInt64Value, MaxInt64Value: Int64;
    MinQWordValue, MaxQWordValue: QWord;
    FloatType: TFloatType;
    { A class: its VMT, its parent's type information, its published
      properties (own and inherited) and its own ones. }
    ClassType: Pointer;
    ParentInfo: PTypeInfo;
    PropCount: SmallInt;
    UnitName: ShortString;
    OwnPropCount: LongInt;
    OwnProps: Pointer;
    { A class's own published fields (of class types) and methods. }
    OwnFieldCount: LongInt;
    OwnFields: Pointer;
    OwnMethodCount: LongInt;
    OwnMethods: Pointer;
  end;

  PVmtFieldEntry = ^TVmtFieldEntry;
  TVmtFieldEntry = record
    FieldOffset: PtrUInt;
    Name: ShortString;
  end;

  PVmtMethodEntry = ^TVmtMethodEntry;
  TVmtMethodEntry = record
    Name: PShortString;
    CodeAddress: Pointer;
  end;

  PPropInfo = ^TPropInfo;
  TPropInfo = record
    PropType: PTypeInfo;
    { A field's offset (ptField), a method (ptStatic) or a constant
      (ptConst), as PropProcs says: bits 0-1 for GetProc, 2-3 SetProc,
      4-5 StoredProc. }
    GetProc, SetProc, StoredProc: Pointer;
    Index: LongInt;
    Default: LongInt;
    NameIndex: SmallInt;
    PropProcs: Byte;
    Name: ShortString;
  end;

{ The C library's heap, which Bruto allocates from: on macOS the default
  malloc zone's statistics; elsewhere the figures are zero. }
function GetFPCHeapStatus: TFPCHeapStatus;
{ Bruto's AnsiStrings carry no code page: they are all UTF-8 (65001), and
  setting one changes nothing. }
procedure SetCodePage(var S: RawByteString; CodePage: TSystemCodePage; Convert: Boolean);
function StringCodePage(const S: RawByteString): TSystemCodePage;

{ The ratio of a circle's circumference to its diameter. }
function Pi: Double;

var
  { The random generator's seed: setting it restarts the sequence. }
  RandSeed: LongWord;

{ FPC 3.2's generator (MT19937), giving the same numbers for a seed:
  Random(L) (0 <= it < L) and Random (0 <= it < 1) call these. }
function BrutoRandomInt(L: LongInt): LongInt;
function BrutoRandomFloat: Double;
{ A seed from the clock. }
procedure Randomize;

{ A Currency's text as Write(C:Len:F) gives it (C its scaled Int64; Len
  -32767 and F -1 when not given), FPC's fpc_shortstr_currency. }
function bruto_curr_str(C: Int64; Len, F: Int64): AnsiString;
{ The Currency product of A and B (scaled Int64s): (A * B) div 10000 on
  a 128-bit product, truncated, as FPC's fpc_mul_currency computes it
  with 64-bit operations only. }
function bruto_curr_mul(A, B: Int64): Int64;
{ S with ASCII letters in lower case, as FPC's System.LowerCase. }
function LowerCase(const S: AnsiString): AnsiString;
{ Compare Len bytes: negative, zero or positive, as FPC. }
function CompareByte(const Buf1, Buf2; Len: SizeInt): SizeInt;
{ The index of the first byte B among Len bytes of Buf, or -1. }
function IndexByte(const Buf; Len: SizeInt; B: Byte): SizeInt;

const
  varEmpty = 0;
  varNull = 1;
  varSmallInt = 2;
  varInteger = 3;
  varSingle = 4;
  varDouble = 5;
  varCurrency = 6;
  varDate = 7;
  varOleStr = 8;
  varDispatch = 9;
  varError = 10;
  varBoolean = 11;
  varVariant = 12;
  varUnknown = 13;
  varDecimal = 14;
  varShortInt = 16;
  varByte = 17;
  varWord = 18;
  varLongWord = 19;
  varInt64 = 20;
  varQWord = 21;
  varRecord = 36;
  varStrArg = $48;
  varAny = $101;
  varString = $100;
  varUString = $102;
  varTypeMask = $FFF;
  varArray = $2000;
  varByRef = $4000;

implementation

function bruto_curr_mul(A, B: Int64): Int64;
var
  Neg: Boolean;
  UA, UB, ALo, AHi, BLo, BHi, T0, T1, T2, T3, Mid, Lo, Hi: QWord;
  Rem, Cur, Q1, Q0: QWord;
begin
  Neg := (A < 0) <> (B < 0);
  if A < 0 then
    UA := QWord(0) - QWord(A)
  else
    UA := QWord(A);
  if B < 0 then
    UB := QWord(0) - QWord(B)
  else
    UB := QWord(B);
  ALo := UA and $FFFFFFFF;
  AHi := UA shr 32;
  BLo := UB and $FFFFFFFF;
  BHi := UB shr 32;
  T0 := ALo * BLo;
  T1 := ALo * BHi;
  T2 := AHi * BLo;
  T3 := AHi * BHi;
  Mid := (T0 shr 32) + (T1 and $FFFFFFFF) + (T2 and $FFFFFFFF);
  Lo := (T0 and $FFFFFFFF) or (Mid shl 32);
  Hi := T3 + (T1 shr 32) + (T2 shr 32) + (Mid shr 32);
  { (Hi:Lo) div 10000, 32 bits at a time; only the low 64 bits of the
    quotient are kept. }
  Cur := Hi shr 32;
  Rem := Cur mod 10000;
  Cur := (Rem shl 32) or (Hi and $FFFFFFFF);
  Rem := Cur mod 10000;
  Cur := (Rem shl 32) or (Lo shr 32);
  Q1 := Cur div 10000;
  Rem := Cur mod 10000;
  Cur := (Rem shl 32) or (Lo and $FFFFFFFF);
  Q0 := Cur div 10000;
  Cur := (Q1 shl 32) or Q0;
  if Neg then
    bruto_curr_mul := -Int64(Cur)
  else
    bruto_curr_mul := Int64(Cur);
end;

function bruto_curr_str(C: Int64; Len, F: Int64): AnsiString;
const
  MinLen = 8;
var
  Buf: array[1..20] of Char;
  S: AnsiString;
  I, J, K, ResLen, TLen, Sign, R, Point: Int64;
  IC: QWord;
begin
  for I := 1 to 20 do
    Buf[I] := '0';
  if Len = -32767 then
    Len := 25;
  if C >= 0 then
  begin
    IC := QWord(C);
    Sign := 0;
  end
  else
  begin
    Sign := 1;
    IC := QWord(-C);
  end;
  TLen := 0;
  repeat
    Inc(TLen);
    Buf[TLen] := Chr(IC mod 10 + 48);
    IC := IC div 10;
  until IC = 0;
  ResLen := TLen;
  if F <> 0 then
    Inc(ResLen);
  if F < 0 then
  begin
    Inc(ResLen, 5);
    if Len < MinLen then
      Len := MinLen;
    R := ResLen - Len;
    if ResLen < Len then
      ResLen := Len;
    if R > 0 then
    begin
      ResLen := Len;
      Point := TLen - R;
    end
    else
      Point := TLen;
  end
  else
  begin
    Inc(ResLen, Sign);
    while TLen < 5 do
    begin
      Inc(ResLen);
      Inc(TLen);
      Buf[TLen] := '0';
    end;
    R := 4 - F;
    Point := F;
    if Point <> 0 then
    begin
      if Point > 4 then
        Point := 4;
      Inc(Point);
    end;
    Dec(ResLen, R);
  end;
  if R > 0 then
  begin
    K := 0;
    I := R + 2;
    if I > TLen then
      I := TLen + 1;
    if Buf[I - 2] >= '5' then
    begin
      if Buf[I - 1] < '9' then
        Buf[I - 1] := Chr(Ord(Buf[I - 1]) + 1)
      else
      begin
        Buf[I - 1] := '0';
        K := 1;
      end;
    end;
    if (K = 1) and (Buf[I - 1] = '0') then
    begin
      while Buf[I] = '9' do
      begin
        Buf[I] := '0';
        Inc(I);
      end;
      Buf[I] := Chr(Ord(Buf[I]) + 1);
      if I > TLen then
      begin
        Inc(ResLen);
        Inc(TLen);
      end;
    end;
  end;
  if ResLen < Len then
    ResLen := Len;
  if ResLen > 255 then
  begin
    if R < 0 then
      Inc(R, ResLen - 255);
    ResLen := 255;
  end;
  SetLength(S, ResLen);
  J := ResLen;
  if F < 0 then
  begin
    if C = 0 then
      K := 0
    else
      K := TLen - 5;
    if K >= 0 then
      S[J - 2] := '+'
    else
    begin
      S[J - 2] := '-';
      K := -K;
    end;
    S[J] := Chr(K mod 10 + 48);
    Dec(J);
    S[J] := Chr(K div 10 + 48);
    Dec(J, 2);
    S[J] := 'E';
    Dec(J);
  end;
  while R < 0 do
  begin
    S[J] := '0';
    Dec(J);
    Inc(R);
  end;
  for I := R + 1 to TLen do
  begin
    Dec(Point);
    if Point = 0 then
    begin
      S[J] := '.';
      Dec(J);
    end;
    S[J] := Buf[I];
    Dec(J);
  end;
  if Sign = 1 then
  begin
    S[J] := '-';
    Dec(J);
  end;
  while J > 0 do
  begin
    S[J] := ' ';
    Dec(J);
  end;
  bruto_curr_str := S;
end;

function Pi: Double;
begin
  Pi := 3.141592653589793238;
end;

const
  BrutoMTN = 624;
  BrutoMTM = 397;

var
  BrutoMT: array[0..BrutoMTN - 1] of Int64;
  BrutoMTI: LongInt;
  BrutoOldRandSeed: LongWord;
  BrutoMTStarted: Boolean;

procedure BrutoMTSeed(Seed: LongWord);
var
  I: LongInt;
  Prev: Int64;
begin
  BrutoMT[0] := Seed;
  for I := 1 to BrutoMTN - 1 do
  begin
    Prev := BrutoMT[I - 1];
    BrutoMT[I] := (1812433253 * (Prev xor (Prev shr 30)) + I) and $FFFFFFFF;
  end;
  BrutoMTI := BrutoMTN;
end;

function BrutoMTNext: LongWord;
var
  Y: Int64;
  K: LongInt;
begin
  { A new seed (or the first call) starts the sequence again. }
  if (not BrutoMTStarted) or (RandSeed <> BrutoOldRandSeed) then
  begin
    BrutoMTSeed(RandSeed);
    RandSeed := not RandSeed;
    BrutoOldRandSeed := RandSeed;
    BrutoMTStarted := True;
  end;
  if BrutoMTI >= BrutoMTN then
  begin
    for K := 0 to BrutoMTN - 1 do
    begin
      Y := (BrutoMT[K] and $80000000) or (BrutoMT[(K + 1) mod BrutoMTN] and $7FFFFFFF);
      BrutoMT[K] := BrutoMT[(K + BrutoMTM) mod BrutoMTN] xor (Y shr 1);
      if (Y and 1) <> 0 then
        BrutoMT[K] := BrutoMT[K] xor $9908B0DF;
    end;
    BrutoMTI := 0;
  end;
  Y := BrutoMT[BrutoMTI];
  BrutoMTI := BrutoMTI + 1;
  Y := Y xor (Y shr 11);
  Y := Y xor ((Y shl 7) and $9D2C5680);
  Y := Y xor ((Y shl 15) and $EFC60000);
  Y := Y xor (Y shr 18);
  BrutoMTNext := Y and $FFFFFFFF;
end;

function BrutoRandomInt(L: LongInt): LongInt;
begin
  if L < 0 then
    L := L + 1;
  BrutoRandomInt := (Int64(BrutoMTNext) * L) div 4294967296;
end;

function BrutoRandomFloat: Double;
begin
  BrutoRandomFloat := BrutoMTNext * (1.0 / 4294967296.0);
end;

function bruto_time(T: Pointer): Int64; cdecl; external 'c' name 'time';
function bruto_clock: Int64; cdecl; external 'c' name 'clock';

procedure Randomize;
begin
  RandSeed := (bruto_time(nil) * 1000 + bruto_clock) and $FFFFFFFF;
end;

{$ifdef DARWIN}
type
  TBrutoMallocStatistics = record
    BlocksInUse, Padding: LongWord;
    SizeInUse, MaxSizeInUse, SizeAllocated: PtrUInt;
  end;

procedure bruto_malloc_zone_statistics(Zone: Pointer; var Stats: TBrutoMallocStatistics);
  cdecl; external 'c' name 'malloc_zone_statistics';
{$endif}

function GetFPCHeapStatus: TFPCHeapStatus;
var
  Status: TFPCHeapStatus;
{$ifdef DARWIN}
  Stats: TBrutoMallocStatistics;
{$endif}
begin
  FillChar(Status, SizeOf(Status), 0);
{$ifdef DARWIN}
  FillChar(Stats, SizeOf(Stats), 0);
  bruto_malloc_zone_statistics(nil, Stats);
  Status.CurrHeapUsed := Stats.SizeInUse;
  Status.CurrHeapSize := Stats.SizeAllocated;
  Status.CurrHeapFree := Stats.SizeAllocated - Stats.SizeInUse;
  Status.MaxHeapUsed := Stats.MaxSizeInUse;
  Status.MaxHeapSize := Stats.SizeAllocated;
{$endif}
  GetFPCHeapStatus := Status;
end;

procedure SetCodePage(var S: RawByteString; CodePage: TSystemCodePage; Convert: Boolean);
begin
end;

function StringCodePage(const S: RawByteString): TSystemCodePage;
begin
  StringCodePage := 65001;
end;

function LowerCase(const S: AnsiString): AnsiString;
var
  I: SizeInt;
  R: AnsiString;
begin
  R := S;
  for I := 1 to Length(R) do
    if (R[I] >= 'A') and (R[I] <= 'Z') then
      R[I] := Chr(Ord(R[I]) + 32);
  LowerCase := R;
end;

function CompareByte(const Buf1, Buf2; Len: SizeInt): SizeInt;
var
  P1, P2: PByte;
  I: SizeInt;
begin
  P1 := @Buf1;
  P2 := @Buf2;
  CompareByte := 0;
  for I := 0 to Len - 1 do
    if P1[I] <> P2[I] then
    begin
      CompareByte := SizeInt(P1[I]) - SizeInt(P2[I]);
      Exit;
    end;
end;

function IndexByte(const Buf; Len: SizeInt; B: Byte): SizeInt;
var
  P: PByte;
  I: SizeInt;
begin
  P := @Buf;
  IndexByte := -1;
  for I := 0 to Len - 1 do
    if P[I] = B then
    begin
      IndexByte := I;
      Exit;
    end;
end;

end.
