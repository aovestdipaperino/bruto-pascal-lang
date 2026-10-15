{ Bruto Pascal runtime: strings. Code generation calls these routines for
  AnsiString, UnicodeString and WideChar operations (and Val); they are
  added to every program, with FPC's UTF-8 conversion routines.

  An AnsiString is a pointer to its characters, followed by a zero byte
  and preceded by a 16-byte header: a marker, a reference count (-1 for
  a literal) and the length, as FPC lays them out. It can hold #0, and
  the pointer is a C string too. A C string from elsewhere (the C
  library, a PChar) has no header; the routines recognise the marker and
  measure those with strlen. The 16 bytes before a pointer are only read
  when they are on the pointer's own memory page (a C string can start a
  page whose predecessor isn't mapped), so a Bruto string never starts in
  the first 16 bytes of a page.

  A UnicodeString is a pointer to UTF-16 code units, followed by a zero
  unit and preceded by the same header (the length counts units), as in
  FPC; both kinds are reference counted the same way.

  Nil is the empty string of either kind. A string is never changed in
  place: every operation that changes one builds a copy. String
  (AnsiString) is UTF-8, as FPC's default code page on Unix. }
unit BrutoStrings;

interface

type
  BrutoUnits = array[0..$FFFFFFF] of Word;
  PBrutoUnits = ^BrutoUnits;
  BrutoBytes = array[0..$FFFFFFF] of Byte;
  PBrutoBytes = ^BrutoBytes;
  PBrutoLength = ^Int64;
  PBrutoMarker = ^LongWord;

  BrutoPointers = array[0..$FFFFFFF] of Pointer;
  PBrutoPointers = ^BrutoPointers;
  PBrutoCount = ^LongInt;

const
  { The header marker of the AnsiStrings Bruto makes, with two flags:
    the string waits in the pool of temporaries, and its block starts 16
    bytes before the header. }
  BrutoAStrMarker = $B57A57A1;
  BrutoAStrPooled = 2;
  BrutoAStrShifted = 4;
  BrutoAStrFlags = 6;

var
  { Strings made by the statement being run and not yet stored anywhere
    (reference count 0): freed when it ends, unless stored by then. }
  BrutoPool: PBrutoPointers;
  BrutoPoolCount, BrutoPoolCap: Int64;
  BrutoArc: Boolean;

function bruto_astr_len(S: Pointer): Int64;
function bruto_astr_alloc(Len: Int64): Pointer;
function bruto_astr_from_buf(P: Pointer; Len: Int64): Pointer;
function bruto_astr_from_char(C: Int64): Pointer;
{ A string holding the C string P, whose memory (from malloc) is freed. }
function bruto_astr_adopt(P: Pointer): Pointer;
{ A copy of the C string at P (nil: the empty string). }
function bruto_astr_from_pchar(P: Pointer): Pointer;
function bruto_astr_concat(A, B: Pointer): Pointer;
function bruto_astr_compare(A, B: Pointer): Int64;
function bruto_astr_copy(S: Pointer; Index, Count: Int64): Pointer;
function bruto_astr_pos(Sub, S: Pointer; Offset: Int64): Int64;
function bruto_astr_delete(S: Pointer; Index, Count: Int64): Pointer;
function bruto_astr_insert(Src, S: Pointer; Index: Int64): Pointer;
function bruto_astr_setlength(S: Pointer; Len: Int64): Pointer;
function bruto_astr_setchar(S: Pointer; Index, C: Int64): Pointer;
{ Reference counting (see the unit's header): a Bruto string's count goes
  up when it is stored and down when it is replaced or goes out of scope;
  at 0 it is freed. C strings and literals are left alone. }
procedure bruto_arc_enable;
procedure bruto_astr_addref(S: Pointer);
procedure bruto_astr_release(S: Pointer);
{ A function's result, released by the function: back to the pool, so
  the caller's statement frees it unless it is stored. }
procedure bruto_astr_return(S: Pointer);
function bruto_pool_mark: Int64;
procedure bruto_pool_drain(Mark: Int64);
{ Write S to the console (and the IDE's capture), #0 characters included. }
procedure bruto_astr_write(S: Pointer);

function bruto_ustr_length(S: Pointer): Int64;
function bruto_ustr_alloc(Len: Int64): Pointer;
function bruto_utf8_to_ustr(S: Pointer): Pointer;
function bruto_ustr_to_utf8(S: Pointer): Pointer;
function bruto_wchar_to_ustr(C: Int64): Pointer;
function bruto_wchar_to_utf8(C: Int64): Pointer;
function bruto_utf8_first_wchar(S: Pointer): Int64;
function bruto_ustr_from_units(P: Pointer; Len: Int64): Pointer;
function bruto_ustr_concat(A, B: Pointer): Pointer;
function bruto_ustr_compare(A, B: Pointer): Int64;
function bruto_ustr_copy(S: Pointer; Index, Count: Int64): Pointer;
function bruto_ustr_pos(Sub, S: Pointer; Offset: Int64): Int64;
function bruto_ustr_delete(S: Pointer; Index, Count: Int64): Pointer;
function bruto_ustr_insert(Src, S: Pointer; Index: Int64): Pointer;
function bruto_ustr_setlength(S: Pointer; Len: Int64): Pointer;
function bruto_ustr_setchar(S: Pointer; Index, C: Int64): Pointer;
function bruto_wchar_upcase(C: Int64): Int64;
function bruto_wchar_lowercase(C: Int64): Int64;
function bruto_ustr_upcase(S: Pointer): Pointer;
function bruto_ustr_lowercase(S: Pointer): Pointer;
{ Val for integers and reals: the value, and in Code the 1-based position
  of the first character that isn't part of the number (0 when all of S
  is), as FPC. }
function bruto_val_int(S: Pointer; var Code: Int64): Int64;
function bruto_val_real(S: Pointer; var Code: Int64): Double;

{ FPC's System routines for UTF-8 and UTF-16. }
{ UTF-8 to UTF-16 into Dest (at most MaxDestChars units), or only counted
  when Dest is nil. As in FPC, the result is the number of units plus one
  for the terminating zero. }
function Utf8ToUnicode(Dest: PUnicodeChar; MaxDestChars: SizeUInt; Source: PChar;
  SourceBytes: SizeUInt): SizeUInt;
{ UTF-16 to UTF-8, the same way (the result counts the terminating zero,
  which is written when there is room). }
function UnicodeToUtf8(Dest: PChar; MaxDestBytes: SizeUInt; Source: PUnicodeChar;
  SourceChars: SizeUInt): SizeUInt;
function UTF8ToString(const S: RawByteString): UnicodeString;
function UTF8Decode(const S: RawByteString): UnicodeString;
function UTF8Encode(const S: UnicodeString): RawByteString;
{ N copies of C. }
function StringOfChar(C: Char; N: Int64): AnsiString;

implementation

function bruto_towupper(C: LongInt): LongInt; cdecl; external 'c' name 'towupper';
function bruto_towlower(C: LongInt): LongInt; cdecl; external 'c' name 'towlower';
function bruto_strtod(S: Pointer; EndPtr: Pointer): Double; cdecl; external 'c' name 'strtod';
function bruto_strlen(S: Pointer): Int64; cdecl; external 'c' name 'strlen';

{ ── AnsiString ────────────────────────────────────────────── }

{ Whether S is a string Bruto made (it has a header). }
function bruto_astr_is_managed(S: Pointer): Boolean;
begin
  bruto_astr_is_managed := (S <> nil) and ((PtrUInt(S) and 4095) >= 16)
    and ((PBrutoMarker(S - 16)^ and not LongWord(BrutoAStrFlags)) = BrutoAStrMarker);
end;

function bruto_astr_len(S: Pointer): Int64;
begin
  if S = nil then
    bruto_astr_len := 0
  else if bruto_astr_is_managed(S) then
    bruto_astr_len := PBrutoLength(S - 8)^
  else
    bruto_astr_len := bruto_strlen(S);
end;

procedure bruto_arc_enable;
begin
  BrutoArc := True;
end;

procedure bruto_pool_add(S: Pointer);
var
  Bigger: PBrutoPointers;
begin
  if BrutoPoolCount = BrutoPoolCap then
  begin
    if BrutoPoolCap = 0 then
      BrutoPoolCap := 256
    else
      BrutoPoolCap := BrutoPoolCap * 2;
    GetMem(Bigger, BrutoPoolCap * SizeOf(Pointer));
    if BrutoPoolCount > 0 then
      Move(BrutoPool^, Bigger^, BrutoPoolCount * SizeOf(Pointer));
    if BrutoPool <> nil then
      FreeMem(BrutoPool);
    BrutoPool := Bigger;
  end;
  BrutoPool^[BrutoPoolCount] := S;
  BrutoPoolCount := BrutoPoolCount + 1;
  PBrutoMarker(S - 16)^ := PBrutoMarker(S - 16)^ or BrutoAStrPooled;
end;

procedure bruto_astr_free(S: Pointer);
begin
  if (PBrutoMarker(S - 16)^ and BrutoAStrShifted) <> 0 then
    FreeMem(S - 32)
  else
    FreeMem(S - 16);
end;

procedure bruto_astr_addref(S: Pointer);
var
  Count: PBrutoCount;
begin
  if not bruto_astr_is_managed(S) then
    Exit;
  Count := PBrutoCount(S - 12);
  if Count^ >= 0 then
    Count^ := Count^ + 1;
end;

procedure bruto_astr_release(S: Pointer);
var
  Count: PBrutoCount;
begin
  if not bruto_astr_is_managed(S) then
    Exit;
  Count := PBrutoCount(S - 12);
  if Count^ <= 0 then
    Exit;
  Count^ := Count^ - 1;
  if (Count^ = 0) and ((PBrutoMarker(S - 16)^ and BrutoAStrPooled) = 0) then
    bruto_astr_free(S);
end;

procedure bruto_astr_return(S: Pointer);
var
  Count: PBrutoCount;
begin
  if not bruto_astr_is_managed(S) then
    Exit;
  Count := PBrutoCount(S - 12);
  if Count^ <= 0 then
    Exit;
  Count^ := Count^ - 1;
  if (Count^ = 0) and ((PBrutoMarker(S - 16)^ and BrutoAStrPooled) = 0) then
    bruto_pool_add(S);
end;

function bruto_pool_mark: Int64;
begin
  bruto_pool_mark := BrutoPoolCount;
end;

procedure bruto_pool_drain(Mark: Int64);
var
  I: Int64;
  S: Pointer;
begin
  I := Mark;
  while I < BrutoPoolCount do
  begin
    S := BrutoPool^[I];
    if PBrutoCount(S - 12)^ = 0 then
      bruto_astr_free(S)
    else
      PBrutoMarker(S - 16)^ := PBrutoMarker(S - 16)^ and not LongWord(BrutoAStrPooled);
    I := I + 1;
  end;
  if Mark < BrutoPoolCount then
    BrutoPoolCount := Mark;
end;

{ A string block: a header (count 0, length Len) and Size zero bytes
  after it, of which a pointer to the first is returned. The data is
  moved 16 bytes further when it would start a page. }
function bruto_string_block(Len, Size: Int64): Pointer;
var
  Block, Header: Pointer;
begin
  GetMem(Block, 32 + Size);
  FillChar(Block^, 32 + Size, 0);
  Header := Block;
  PBrutoMarker(Header)^ := BrutoAStrMarker;
  if ((PtrUInt(Block) + 16) and 4095) < 16 then
  begin
    Header := Block + 16;
    PBrutoMarker(Header)^ := BrutoAStrMarker or BrutoAStrShifted;
  end;
  PBrutoLength(Header + 8)^ := Len;
  if BrutoArc then
    bruto_pool_add(Header + 16);
  bruto_string_block := Header + 16;
end;

{ A string of Len bytes, all zero (the reference count 0). }
function bruto_astr_alloc(Len: Int64): Pointer;
begin
  if Len < 0 then
    Len := 0;
  bruto_astr_alloc := bruto_string_block(Len, Len + 1);
end;

function bruto_astr_from_pchar(P: Pointer): Pointer;
begin
  if P = nil then
    bruto_astr_from_pchar := nil
  else
    bruto_astr_from_pchar := bruto_astr_from_buf(P, bruto_strlen(P));
end;

function bruto_astr_from_buf(P: Pointer; Len: Int64): Pointer;
var
  R: Pointer;
begin
  R := bruto_astr_alloc(Len);
  if Len > 0 then
    Move(P^, R^, Len);
  bruto_astr_from_buf := R;
end;

function bruto_astr_from_char(C: Int64): Pointer;
var
  R: PBrutoBytes;
begin
  R := PBrutoBytes(bruto_astr_alloc(1));
  R^[0] := C;
  bruto_astr_from_char := R;
end;

function bruto_astr_adopt(P: Pointer): Pointer;
begin
  if P = nil then
  begin
    bruto_astr_adopt := bruto_astr_alloc(0);
    Exit;
  end;
  bruto_astr_adopt := bruto_astr_from_buf(P, bruto_strlen(P));
  FreeMem(P);
end;

function bruto_astr_concat(A, B: Pointer): Pointer;
var
  LA, LB: Int64;
  R: PBrutoBytes;
begin
  LA := bruto_astr_len(A);
  LB := bruto_astr_len(B);
  R := PBrutoBytes(bruto_astr_alloc(LA + LB));
  if LA > 0 then
    Move(A^, R^[0], LA);
  if LB > 0 then
    Move(B^, R^[LA], LB);
  bruto_astr_concat := R;
end;

{ Byte order, then length: negative, zero or positive. }
function bruto_astr_compare(A, B: Pointer): Int64;
var
  LA, LB, I: Int64;
  BA, BB: PBrutoBytes;
begin
  LA := bruto_astr_len(A);
  LB := bruto_astr_len(B);
  BA := PBrutoBytes(A);
  BB := PBrutoBytes(B);
  I := 0;
  while (I < LA) and (I < LB) do
  begin
    if BA^[I] <> BB^[I] then
    begin
      bruto_astr_compare := Int64(BA^[I]) - Int64(BB^[I]);
      Exit;
    end;
    I := I + 1;
  end;
  bruto_astr_compare := LA - LB;
end;

{ As FPC: an index below 1 counts as 1, and the count is cut to what the
  string holds. }
function bruto_astr_copy(S: Pointer; Index, Count: Int64): Pointer;
var
  Len: Int64;
begin
  Len := bruto_astr_len(S);
  if Index < 1 then
    Index := 1;
  if Index > Len then
    Count := 0
  else if Count > Len - Index + 1 then
    Count := Len - Index + 1;
  if Count <= 0 then
    bruto_astr_copy := bruto_astr_alloc(0)
  else
    bruto_astr_copy := bruto_astr_from_buf(S + (Index - 1), Count);
end;

function bruto_astr_pos(Sub, S: Pointer; Offset: Int64): Int64;
var
  LS, LSub, I, J: Int64;
  BS, BSub: PBrutoBytes;
  Found: Boolean;
begin
  bruto_astr_pos := 0;
  LS := bruto_astr_len(S);
  LSub := bruto_astr_len(Sub);
  if (LSub = 0) or (Offset < 1) then
    Exit;
  BS := PBrutoBytes(S);
  BSub := PBrutoBytes(Sub);
  for I := Offset - 1 to LS - LSub do
  begin
    Found := True;
    J := 0;
    while Found and (J < LSub) do
    begin
      if BS^[I + J] <> BSub^[J] then
        Found := False;
      J := J + 1;
    end;
    if Found then
    begin
      bruto_astr_pos := I + 1;
      Exit;
    end;
  end;
end;

function bruto_astr_delete(S: Pointer; Index, Count: Int64): Pointer;
var
  Len: Int64;
begin
  Len := bruto_astr_len(S);
  if (Index < 1) or (Index > Len) or (Count <= 0) then
  begin
    bruto_astr_delete := S;
    Exit;
  end;
  if Count > Len - Index + 1 then
    Count := Len - Index + 1;
  bruto_astr_delete := bruto_astr_concat(bruto_astr_copy(S, 1, Index - 1),
    bruto_astr_copy(S, Index + Count, Len));
end;

function bruto_astr_insert(Src, S: Pointer; Index: Int64): Pointer;
var
  Len: Int64;
begin
  Len := bruto_astr_len(S);
  if Index < 1 then
    Index := 1;
  if Index > Len + 1 then
    Index := Len + 1;
  bruto_astr_insert := bruto_astr_concat(
    bruto_astr_concat(bruto_astr_copy(S, 1, Index - 1), Src),
    bruto_astr_copy(S, Index, Len));
end;

{ New characters are #0, as FPC's memory usually holds. }
function bruto_astr_setlength(S: Pointer; Len: Int64): Pointer;
var
  Old: Int64;
  R: Pointer;
begin
  Old := bruto_astr_len(S);
  R := bruto_astr_alloc(Len);
  if Old > Len then
    Old := Len;
  if Old > 0 then
    Move(S^, R^, Old);
  bruto_astr_setlength := R;
end;

{ S with byte Index (1-based) set to C, as a new string. }
function bruto_astr_setchar(S: Pointer; Index, C: Int64): Pointer;
var
  Len: Int64;
  R: PBrutoBytes;
begin
  Len := bruto_astr_len(S);
  R := PBrutoBytes(bruto_astr_from_buf(S, Len));
  if (Index >= 1) and (Index <= Len) then
    R^[Index - 1] := C;
  bruto_astr_setchar := R;
end;

procedure bruto_rt_write_str(S: Pointer); cdecl; external 'c' name 'bruto_write_str';
procedure bruto_rt_capture_write_str(S: Pointer); cdecl; external 'c' name 'bruto_capture_write_str';
procedure bruto_rt_write_char(C: Char); cdecl; external 'c' name 'bruto_write_char';
procedure bruto_rt_capture_write_char(C: Char); cdecl; external 'c' name 'bruto_capture_write_char';

procedure bruto_astr_write(S: Pointer);
var
  Len, I: Int64;
  B: PBrutoBytes;
begin
  if S = nil then
    Exit;
  Len := bruto_astr_len(S);
  if bruto_strlen(S) = Len then
  begin
    bruto_rt_write_str(S);
    bruto_rt_capture_write_str(S);
    Exit;
  end;
  B := PBrutoBytes(S);
  for I := 0 to Len - 1 do
  begin
    bruto_rt_write_char(Chr(B^[I]));
    bruto_rt_capture_write_char(Chr(B^[I]));
  end;
end;

{ ── UnicodeString ─────────────────────────────────────────── }


function bruto_ustr_length(S: Pointer): Int64;
begin
  if S = nil then
    bruto_ustr_length := 0
  else
    bruto_ustr_length := PBrutoLength(S - 8)^;
end;

{ A string of Len units, all zero, or nil when Len is 0. }
function bruto_ustr_alloc(Len: Int64): Pointer;
begin
  if Len <= 0 then
    bruto_ustr_alloc := nil
  else
    bruto_ustr_alloc := bruto_string_block(Len, (Len + 1) * 2);
end;

{ The length of the UTF-8 sequence starting with byte B, or 0 when B
  can't start one. }
function bruto_utf8_seqlen(B: Byte): Int64;
begin
  if B < $80 then
    bruto_utf8_seqlen := 1
  else if (B and $E0) = $C0 then
    bruto_utf8_seqlen := 2
  else if (B and $F0) = $E0 then
    bruto_utf8_seqlen := 3
  else if (B and $F8) = $F0 then
    bruto_utf8_seqlen := 4
  else
    bruto_utf8_seqlen := 0;
end;

{ Decode the character at S[I] into Code, returning its length in bytes.
  A byte that doesn't start a well-formed sequence is taken as a Latin-1
  character, so Pascal's byte strings (#196 is 'Ä') convert too. }
function bruto_utf8_decode(S: PBrutoBytes; I: Int64; var Code: Int64): Int64;
var
  N, K: Int64;
  B: Byte;
begin
  B := S^[I];
  N := bruto_utf8_seqlen(B);
  if N <= 1 then
  begin
    Code := B;
    bruto_utf8_decode := 1;
    Exit;
  end;
  if N = 2 then
    Code := B and $1F
  else if N = 3 then
    Code := B and $0F
  else
    Code := B and $07;
  for K := 1 to N - 1 do
  begin
    if (S^[I + K] and $C0) <> $80 then
    begin
      Code := B;
      bruto_utf8_decode := 1;
      Exit;
    end;
    Code := (Code shl 6) or (S^[I + K] and $3F);
  end;
  bruto_utf8_decode := N;
end;

function bruto_utf16_units(Code: Int64): Int64;
begin
  if Code > $FFFF then
    bruto_utf16_units := 2
  else
    bruto_utf16_units := 1;
end;

function bruto_utf8_to_ustr(S: Pointer): Pointer;
var
  Bytes: PBrutoBytes;
  I, Len, Code, ByteLen: Int64;
  R: PBrutoUnits;
begin
  bruto_utf8_to_ustr := nil;
  if S = nil then
    Exit;
  Bytes := PBrutoBytes(S);
  ByteLen := bruto_astr_len(S);
  Len := 0;
  I := 0;
  while I < ByteLen do
  begin
    I := I + bruto_utf8_decode(Bytes, I, Code);
    Len := Len + bruto_utf16_units(Code);
  end;
  if Len = 0 then
    Exit;
  R := PBrutoUnits(bruto_ustr_alloc(Len));
  Len := 0;
  I := 0;
  while I < ByteLen do
  begin
    I := I + bruto_utf8_decode(Bytes, I, Code);
    if Code > $FFFF then
    begin
      Code := Code - $10000;
      R^[Len] := $D800 + (Code shr 10);
      R^[Len + 1] := $DC00 + (Code and $3FF);
      Len := Len + 2;
    end
    else
    begin
      R^[Len] := Code;
      Len := Len + 1;
    end;
  end;
  bruto_utf8_to_ustr := R;
end;

{ The code point starting at unit I of S (of length Len), and in Units
  how many units it takes. }
function bruto_utf16_code(S: PBrutoUnits; I, Len: Int64; var Units: Int64): Int64;
var
  Hi, Lo: Int64;
begin
  Hi := S^[I];
  Units := 1;
  bruto_utf16_code := Hi;
  if (Hi >= $D800) and (Hi < $DC00) and (I + 1 < Len) then
  begin
    Lo := S^[I + 1];
    if (Lo >= $DC00) and (Lo < $E000) then
    begin
      bruto_utf16_code := $10000 + ((Hi - $D800) shl 10) + (Lo - $DC00);
      Units := 2;
    end;
  end;
end;

function bruto_utf8_bytes(Code: Int64): Int64;
begin
  if Code < $80 then
    bruto_utf8_bytes := 1
  else if Code < $800 then
    bruto_utf8_bytes := 2
  else if Code < $10000 then
    bruto_utf8_bytes := 3
  else
    bruto_utf8_bytes := 4;
end;

function bruto_utf8_encode(R: PBrutoBytes; J, Code: Int64): Int64;
begin
  if Code < $80 then
  begin
    R^[J] := Code;
    bruto_utf8_encode := 1;
  end
  else if Code < $800 then
  begin
    R^[J] := $C0 or (Code shr 6);
    R^[J + 1] := $80 or (Code and $3F);
    bruto_utf8_encode := 2;
  end
  else if Code < $10000 then
  begin
    R^[J] := $E0 or (Code shr 12);
    R^[J + 1] := $80 or ((Code shr 6) and $3F);
    R^[J + 2] := $80 or (Code and $3F);
    bruto_utf8_encode := 3;
  end
  else
  begin
    R^[J] := $F0 or (Code shr 18);
    R^[J + 1] := $80 or ((Code shr 12) and $3F);
    R^[J + 2] := $80 or ((Code shr 6) and $3F);
    R^[J + 3] := $80 or (Code and $3F);
    bruto_utf8_encode := 4;
  end;
end;

function bruto_ustr_to_utf8(S: Pointer): Pointer;
var
  U: PBrutoUnits;
  R: PBrutoBytes;
  I, J, Len, Size, Units, Code: Int64;
begin
  U := PBrutoUnits(S);
  Len := bruto_ustr_length(S);
  Size := 0;
  I := 0;
  while I < Len do
  begin
    Code := bruto_utf16_code(U, I, Len, Units);
    Size := Size + bruto_utf8_bytes(Code);
    I := I + Units;
  end;
  R := PBrutoBytes(bruto_astr_alloc(Size));
  I := 0;
  J := 0;
  while I < Len do
  begin
    Code := bruto_utf16_code(U, I, Len, Units);
    J := J + bruto_utf8_encode(R, J, Code);
    I := I + Units;
  end;
  bruto_ustr_to_utf8 := R;
end;

function bruto_wchar_to_ustr(C: Int64): Pointer;
var
  R: PBrutoUnits;
begin
  R := PBrutoUnits(bruto_ustr_alloc(1));
  R^[0] := C;
  bruto_wchar_to_ustr := R;
end;

function bruto_wchar_to_utf8(C: Int64): Pointer;
var
  R: PBrutoBytes;
begin
  R := PBrutoBytes(bruto_astr_alloc(4));
  PBrutoLength(Pointer(R) - 8)^ := bruto_utf8_encode(R, 0, C);
  bruto_wchar_to_utf8 := R;
end;

{ A one-character string literal ('a', or a CJK character from a UTF-8
  source) used as a WideChar. }
function bruto_utf8_first_wchar(S: Pointer): Int64;
var
  Code: Int64;
begin
  bruto_utf8_first_wchar := 0;
  if S = nil then
    Exit;
  if PBrutoBytes(S)^[0] = 0 then
    Exit;
  bruto_utf8_decode(PBrutoBytes(S), 0, Code);
  if Code > $FFFF then
    Code := $D800 + ((Code - $10000) shr 10);
  bruto_utf8_first_wchar := Code;
end;

{ A string of the Len units at P (a literal's UTF-16 data). }
function bruto_ustr_from_units(P: Pointer; Len: Int64): Pointer;
var
  R: PBrutoUnits;
begin
  R := PBrutoUnits(bruto_ustr_alloc(Len));
  if Len > 0 then
    Move(P^, R^[0], Len * 2);
  bruto_ustr_from_units := R;
end;

function bruto_ustr_concat(A, B: Pointer): Pointer;
var
  LA, LB: Int64;
  R: PBrutoUnits;
begin
  LA := bruto_ustr_length(A);
  LB := bruto_ustr_length(B);
  R := PBrutoUnits(bruto_ustr_alloc(LA + LB));
  if LA > 0 then
    Move(A^, R^[0], LA * 2);
  if LB > 0 then
    Move(B^, R^[LA], LB * 2);
  bruto_ustr_concat := R;
end;

function bruto_ustr_compare(A, B: Pointer): Int64;
var
  LA, LB, I: Int64;
  UA, UB: PBrutoUnits;
begin
  LA := bruto_ustr_length(A);
  LB := bruto_ustr_length(B);
  UA := PBrutoUnits(A);
  UB := PBrutoUnits(B);
  I := 0;
  while (I < LA) and (I < LB) do
  begin
    if UA^[I] <> UB^[I] then
    begin
      bruto_ustr_compare := Int64(UA^[I]) - Int64(UB^[I]);
      Exit;
    end;
    I := I + 1;
  end;
  bruto_ustr_compare := LA - LB;
end;

function bruto_ustr_copy(S: Pointer; Index, Count: Int64): Pointer;
var
  Len: Int64;
  R: PBrutoUnits;
begin
  Len := bruto_ustr_length(S);
  if Index < 1 then
    Index := 1;
  if Index > Len then
    Count := 0
  else if Count > Len - Index + 1 then
    Count := Len - Index + 1;
  bruto_ustr_copy := nil;
  if Count <= 0 then
    Exit;
  R := PBrutoUnits(bruto_ustr_alloc(Count));
  Move(PBrutoUnits(S)^[Index - 1], R^[0], Count * 2);
  bruto_ustr_copy := R;
end;

function bruto_ustr_pos(Sub, S: Pointer; Offset: Int64): Int64;
var
  LS, LSub, I, J: Int64;
  US, USub: PBrutoUnits;
  Found: Boolean;
begin
  bruto_ustr_pos := 0;
  LS := bruto_ustr_length(S);
  LSub := bruto_ustr_length(Sub);
  if (LSub = 0) or (Offset < 1) then
    Exit;
  US := PBrutoUnits(S);
  USub := PBrutoUnits(Sub);
  for I := Offset - 1 to LS - LSub do
  begin
    Found := True;
    J := 0;
    while Found and (J < LSub) do
    begin
      if US^[I + J] <> USub^[J] then
        Found := False;
      J := J + 1;
    end;
    if Found then
    begin
      bruto_ustr_pos := I + 1;
      Exit;
    end;
  end;
end;

function bruto_ustr_delete(S: Pointer; Index, Count: Int64): Pointer;
var
  Len: Int64;
begin
  Len := bruto_ustr_length(S);
  if (Index < 1) or (Index > Len) or (Count <= 0) then
  begin
    bruto_ustr_delete := S;
    Exit;
  end;
  if Count > Len - Index + 1 then
    Count := Len - Index + 1;
  bruto_ustr_delete := bruto_ustr_concat(bruto_ustr_copy(S, 1, Index - 1),
    bruto_ustr_copy(S, Index + Count, Len));
end;

function bruto_ustr_insert(Src, S: Pointer; Index: Int64): Pointer;
var
  Len: Int64;
begin
  Len := bruto_ustr_length(S);
  if Index < 1 then
    Index := 1;
  if Index > Len + 1 then
    Index := Len + 1;
  bruto_ustr_insert := bruto_ustr_concat(
    bruto_ustr_concat(bruto_ustr_copy(S, 1, Index - 1), Src),
    bruto_ustr_copy(S, Index, Len));
end;

function bruto_ustr_setlength(S: Pointer; Len: Int64): Pointer;
var
  Old: Int64;
  R: PBrutoUnits;
begin
  Old := bruto_ustr_length(S);
  R := PBrutoUnits(bruto_ustr_alloc(Len));
  if Old > Len then
    Old := Len;
  if Old > 0 then
    Move(S^, R^[0], Old * 2);
  bruto_ustr_setlength := R;
end;

{ S with unit Index (1-based) set to C, as a new string. }
function bruto_ustr_setchar(S: Pointer; Index, C: Int64): Pointer;
var
  Len: Int64;
  R: PBrutoUnits;
begin
  Len := bruto_ustr_length(S);
  R := PBrutoUnits(bruto_ustr_alloc(Len));
  if Len > 0 then
    Move(S^, R^[0], Len * 2);
  if (Index >= 1) and (Index <= Len) then
    R^[Index - 1] := C;
  bruto_ustr_setchar := R;
end;

{ Case mapping for ASCII, Latin-1, Latin Extended-A, Greek and Cyrillic;
  in the C locale the C library maps ASCII only. }
function bruto_wchar_upcase(C: Int64): Int64;
begin
  if (C >= Ord('a')) and (C <= Ord('z')) then
    C := C - 32
  else if (C >= $E0) and (C <= $FE) and (C <> $F7) then
    C := C - 32
  else if C = $FF then
    C := $178
  else if (C >= $100) and (C <= $17F) and (C <> $130) and (C <> $131) and (C <> $138)
    and (C <> $149) and (C <> $17F) then
  begin
    if ((C >= $139) and (C <= $148)) or ((C >= $179) and (C <= $17E)) then
    begin
      if (C and 1) = 0 then
        C := C - 1;
    end
    else if (C and 1) = 1 then
      C := C - 1;
  end
  else if (C >= $3B1) and (C <= $3C9) and (C <> $3C2) then
    C := C - 32
  else if (C >= $430) and (C <= $44F) then
    C := C - 32
  else if (C >= $450) and (C <= $45F) then
    C := C - 80
  else if C > $7F then
    C := bruto_towupper(C) and $FFFF;
  bruto_wchar_upcase := C;
end;

function bruto_wchar_lowercase(C: Int64): Int64;
begin
  if (C >= Ord('A')) and (C <= Ord('Z')) then
    C := C + 32
  else if (C >= $C0) and (C <= $DE) and (C <> $D7) then
    C := C + 32
  else if C = $178 then
    C := $FF
  else if (C >= $100) and (C <= $17F) and (C <> $130) and (C <> $131) and (C <> $138)
    and (C <> $149) and (C <> $17F) then
  begin
    if ((C >= $139) and (C <= $148)) or ((C >= $179) and (C <= $17E)) then
    begin
      if (C and 1) = 1 then
        C := C + 1;
    end
    else if (C and 1) = 0 then
      C := C + 1;
  end
  else if (C >= $391) and (C <= $3A9) and (C <> $3A2) then
    C := C + 32
  else if (C >= $410) and (C <= $42F) then
    C := C + 32
  else if (C >= $400) and (C <= $40F) then
    C := C + 80
  else if C > $7F then
    C := bruto_towlower(C) and $FFFF;
  bruto_wchar_lowercase := C;
end;

function bruto_ustr_upcase(S: Pointer): Pointer;
var
  Len, I: Int64;
  R: PBrutoUnits;
begin
  Len := bruto_ustr_length(S);
  R := PBrutoUnits(bruto_ustr_alloc(Len));
  for I := 0 to Len - 1 do
    R^[I] := bruto_wchar_upcase(PBrutoUnits(S)^[I]);
  bruto_ustr_upcase := R;
end;

function bruto_ustr_lowercase(S: Pointer): Pointer;
var
  Len, I: Int64;
  R: PBrutoUnits;
begin
  Len := bruto_ustr_length(S);
  R := PBrutoUnits(bruto_ustr_alloc(Len));
  for I := 0 to Len - 1 do
    R^[I] := bruto_wchar_lowercase(PBrutoUnits(S)^[I]);
  bruto_ustr_lowercase := R;
end;

function bruto_val_int(S: Pointer; var Code: Int64): Int64;
var
  P: PBrutoBytes;
  I, Base, D, V: Int64;
  Negative, Any: Boolean;
  C: Byte;
begin
  bruto_val_int := 0;
  Code := 1;
  if S = nil then
    Exit;
  P := PBrutoBytes(S);
  I := 0;
  while (P^[I] = 32) or (P^[I] = 9) do
    I := I + 1;
  Negative := False;
  if (P^[I] = Ord('-')) or (P^[I] = Ord('+')) then
  begin
    Negative := P^[I] = Ord('-');
    I := I + 1;
  end;
  Base := 10;
  if P^[I] = Ord('$') then
  begin
    Base := 16;
    I := I + 1;
  end
  else if P^[I] = Ord('%') then
  begin
    Base := 2;
    I := I + 1;
  end
  else if P^[I] = Ord('&') then
  begin
    Base := 8;
    I := I + 1;
  end
  else if (P^[I] = Ord('0')) and ((P^[I + 1] = Ord('x')) or (P^[I + 1] = Ord('X'))) then
  begin
    Base := 16;
    I := I + 2;
  end;
  V := 0;
  Any := False;
  while P^[I] <> 0 do
  begin
    C := P^[I];
    if (C >= Ord('0')) and (C <= Ord('9')) then
      D := C - Ord('0')
    else if (C >= Ord('a')) and (C <= Ord('f')) then
      D := C - Ord('a') + 10
    else if (C >= Ord('A')) and (C <= Ord('F')) then
      D := C - Ord('A') + 10
    else
      D := 99;
    if D >= Base then
    begin
      Code := I + 1;
      Exit;
    end;
    V := V * Base + D;
    Any := True;
    I := I + 1;
  end;
  if not Any then
  begin
    Code := I + 1;
    Exit;
  end;
  if Negative then
    V := -V;
  Code := 0;
  bruto_val_int := V;
end;

function bruto_val_real(S: Pointer; var Code: Int64): Double;
var
  P: PBrutoBytes;
  Stop: Pointer;
  V: Double;
begin
  bruto_val_real := 0;
  Code := 1;
  if S = nil then
    Exit;
  P := PBrutoBytes(S);
  if P^[0] = 0 then
    Exit;
  Stop := nil;
  V := bruto_strtod(S, @Stop);
  if PBrutoBytes(Stop)^[0] <> 0 then
  begin
    Code := (Stop - S) + 1;
    Exit;
  end;
  if Stop = S then
    Exit;
  Code := 0;
  bruto_val_real := V;
end;

function Utf8ToUnicode(Dest: PUnicodeChar; MaxDestChars: SizeUInt; Source: PChar;
  SourceBytes: SizeUInt): SizeUInt;
var
  I, J, N, K, Code: Int64;
  B: Byte;
  Bad: Boolean;
begin
  Utf8ToUnicode := 0;
  if Source = nil then
    Exit;
  I := 0;
  J := 0;
  while I < SourceBytes do
  begin
    B := Ord(Source[I]);
    if B < $80 then
      N := 1
    else if (B and $E0) = $C0 then
      N := 2
    else if (B and $F0) = $E0 then
      N := 3
    else if (B and $F8) = $F0 then
      N := 4
    else
      N := 0;
    Bad := (N = 0) or (I + N > SourceBytes);
    if not Bad then
    begin
      if N = 1 then
        Code := B
      else if N = 2 then
        Code := B and $1F
      else if N = 3 then
        Code := B and $0F
      else
        Code := B and $07;
      K := 1;
      while (not Bad) and (K < N) do
      begin
        if (Ord(Source[I + K]) and $C0) <> $80 then
          Bad := True
        else
          Code := (Code shl 6) or (Ord(Source[I + K]) and $3F);
        K := K + 1;
      end;
    end;
    if Bad then
    begin
      { As FPC: an invalid byte becomes '?'. }
      Code := Ord('?');
      N := 1;
    end;
    if Code > $FFFF then
    begin
      if Dest <> nil then
      begin
        if J + 2 > MaxDestChars then
          Break;
        Dest[J] := WideChar($D800 + ((Code - $10000) shr 10));
        Dest[J + 1] := WideChar($DC00 + ((Code - $10000) and $3FF));
      end;
      J := J + 2;
    end
    else
    begin
      if Dest <> nil then
      begin
        if J + 1 > MaxDestChars then
          Break;
        Dest[J] := WideChar(Code);
      end;
      J := J + 1;
    end;
    I := I + N;
  end;
  if (Dest <> nil) and (J < MaxDestChars) then
    Dest[J] := #0;
  Utf8ToUnicode := J + 1;
end;

function UnicodeToUtf8(Dest: PChar; MaxDestBytes: SizeUInt; Source: PUnicodeChar;
  SourceChars: SizeUInt): SizeUInt;
var
  I, J, N, Code, Lo: Int64;
begin
  UnicodeToUtf8 := 0;
  if Source = nil then
    Exit;
  I := 0;
  J := 0;
  while I < SourceChars do
  begin
    Code := Ord(Source[I]);
    I := I + 1;
    if (Code >= $D800) and (Code < $DC00) and (I < SourceChars) then
    begin
      Lo := Ord(Source[I]);
      if (Lo >= $DC00) and (Lo < $E000) then
      begin
        Code := $10000 + ((Code - $D800) shl 10) + (Lo - $DC00);
        I := I + 1;
      end;
    end;
    if Code < $80 then
      N := 1
    else if Code < $800 then
      N := 2
    else if Code < $10000 then
      N := 3
    else
      N := 4;
    if Dest <> nil then
    begin
      if J + N > MaxDestBytes then
        Break;
      if N = 1 then
        Dest[J] := Chr(Code)
      else if N = 2 then
      begin
        Dest[J] := Chr($C0 or (Code shr 6));
        Dest[J + 1] := Chr($80 or (Code and $3F));
      end
      else if N = 3 then
      begin
        Dest[J] := Chr($E0 or (Code shr 12));
        Dest[J + 1] := Chr($80 or ((Code shr 6) and $3F));
        Dest[J + 2] := Chr($80 or (Code and $3F));
      end
      else
      begin
        Dest[J] := Chr($F0 or (Code shr 18));
        Dest[J + 1] := Chr($80 or ((Code shr 12) and $3F));
        Dest[J + 2] := Chr($80 or ((Code shr 6) and $3F));
        Dest[J + 3] := Chr($80 or (Code and $3F));
      end;
    end;
    J := J + N;
  end;
  if (Dest <> nil) and (J < MaxDestBytes) then
    Dest[J] := #0;
  UnicodeToUtf8 := J + 1;
end;

function UTF8ToString(const S: RawByteString): UnicodeString;
begin
  UTF8ToString := S;
end;

function UTF8Decode(const S: RawByteString): UnicodeString;
begin
  UTF8Decode := S;
end;

function UTF8Encode(const S: UnicodeString): RawByteString;
begin
  UTF8Encode := S;
end;

function StringOfChar(C: Char; N: Int64): AnsiString;
var
  S: AnsiString;
begin
  S := '';
  if N > 0 then
  begin
    SetLength(S, N);
    FillChar(S[1], N, C);
  end;
  StringOfChar := S;
end;

end.
