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

{ Bruto doesn't track heap use: the figures are all zero. }
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

function GetFPCHeapStatus: TFPCHeapStatus;
var
  Status: TFPCHeapStatus;
begin
  FillChar(Status, SizeOf(Status), 0);
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
