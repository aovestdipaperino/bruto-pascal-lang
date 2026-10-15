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

{ The C library's heap, which Bruto allocates from: on macOS the default
  malloc zone's statistics; elsewhere the figures are zero. }
function GetFPCHeapStatus: TFPCHeapStatus;
{ Bruto's AnsiStrings carry no code page: they are all UTF-8 (65001), and
  setting one changes nothing. }
procedure SetCodePage(var S: RawByteString; CodePage: TSystemCodePage; Convert: Boolean);
function StringCodePage(const S: RawByteString): TSystemCodePage;

{ Compare Len bytes: negative, zero or positive, as FPC. }
function CompareByte(const Buf1, Buf2; Len: SizeInt): SizeInt;
{ The index of the first byte B among Len bytes of Buf, or -1. }
function IndexByte(const Buf; Len: SizeInt; B: Byte): SizeInt;

implementation

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
