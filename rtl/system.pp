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

{ S with ASCII letters in lower case, as FPC's System.LowerCase. }
function LowerCase(const S: AnsiString): AnsiString;
{ Compare Len bytes: negative, zero or positive, as FPC. }
function CompareByte(const Buf1, Buf2; Len: SizeInt): SizeInt;
{ The index of the first byte B among Len bytes of Buf, or -1. }
function IndexByte(const Buf; Len: SizeInt; B: Byte): SizeInt;

implementation

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
