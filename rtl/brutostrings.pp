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
  { A dynamic array: the header's count is its number of elements, and the
    two words before the header are the routines that release and count
    a run of its elements (nil when they hold nothing counted). }
  BrutoDynArray = 8;
  BrutoAStrFlags = 14;

type
  { Called with a runtime error's code before the program stops; SysUtils
    raises the matching exception from it. }
  TBrutoRunErrorHook = procedure(Code: LongInt);

var
  BrutoRunErrorHook: TBrutoRunErrorHook;
  { The floating point exceptions that don't stop the program, as Math's
    TFPUExceptionMask bits (invalid operation 1, denormal 2, division by
    zero 4, overflow 8, underflow 16, precision 32), xor-ed with FPC's
    default (denormal, underflow and precision masked). }
  BrutoFPUMaskFlip: LongInt;
  { Strings made by the statement being run and not yet stored anywhere
    (reference count 0): freed when it ends, unless stored by then. }
  BrutoPool: PBrutoPointers;
  BrutoPoolCount, BrutoPoolCap: Int64;
  BrutoArc: Boolean;

function bruto_astr_len(S: Pointer): Int64;
function bruto_astr_alloc(Len: Int64): Pointer;

{ Runtime error Code at Address: with UseHook, SysUtils may raise the
  matching exception; otherwise FPC's report goes to stderr and the
  program stops with exit code Code. }
procedure bruto_runtime_error(Code: LongInt; Address: Pointer; UseHook: Boolean);
{ Report memory access faults (nil pointers...) as runtime error 216, as
  FPC does. }
procedure bruto_install_fault_handlers;

{ Dynamic arrays: a pointer to the first of Count elements of ElemSize
  bytes (zeroed), with the string header before it; nil when empty.
  Cleanup and Addref are procedure(P: Pointer; Count: Int64), or nil. }
function bruto_dynarr_block(Count, ElemSize: Int64; Cleanup, Addref: Pointer): Pointer;
{ SetLength on the dynamic array in Slot: it ends up the only reference
  to a block of NewCount elements, the old ones kept. }
procedure bruto_dynarr_setlength(Slot: Pointer; NewCount, ElemSize: Int64; Cleanup, Addref: Pointer);
{ Count elements of P from Start (0-based), in a new array. }
function bruto_dynarr_copy(P: Pointer; Start, Count, ElemSize: Int64): Pointer;
{ Delete(A, Index, Count) on the dynamic array in Slot. }
procedure bruto_dynarr_delete(Slot: Pointer; Index, Count, ElemSize: Int64);
{ Make room for one element at Index (at most the length) in the
  dynamic array in Slot (its own block afterwards); the index used. }
function bruto_dynarr_insert_gap(Slot: Pointer; Index, ElemSize: Int64; Cleanup, Addref: Pointer): Int64;
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
{ Variants (16 bytes, as FPC's TVarData: a Word type, three reserved
  Words and 8 bytes of data): an AnsiString, a UnicodeString (OleStr)
  or a variant array (a dynamic array of variants) in the data is
  counted like a string. }
procedure bruto_var_addref(P: Pointer);
procedure bruto_var_release(P: Pointer);
procedure bruto_var_return(P: Pointer);
{ A dynamic array block holding its own reference (see bruto_dynarr_block):
  what a variant array's data is made of. }
function bruto_dynarr_owned(Count, ElemSize: Int64; Cleanup, Addref: Pointer): Pointer;
{ Interface references (Obj, ITable): counted through the itable's
  _AddRef and _Release. }
procedure bruto_intf_addref(Obj, ITable: Pointer);
procedure bruto_intf_release(Obj, ITable: Pointer);
{ Obj's reference to the interface described by Desc, stored at Out
  (two words: instance and itable; uncounted): whether there is one.
  A class's interface table holds (descriptor, itable, getter) triples; a
  getter (for a property that implements the interface) gives the
  reference itself. }
function bruto_intf_query(Obj, Desc, Out: Pointer): Boolean;
{ The runtime error a floating point operation Op (1 +, 2 -, 3 *, 4 /,
  5 Sqrt, 6 Ln, 7 Exp) on A (and B) giving R (an infinity or NaN) is, as
  FPC reports it: 208 division by zero, 207 invalid operation, 205
  overflow, or 0 (none, or masked). }
function bruto_float_error(A, B, R: Double; Op: LongInt): LongInt;
{ X as FPC's Str and Write give it: Width (or -1) and FracDigits (or -1,
  for the exponential form), with a Single's 10 digits and 2-digit
  exponent, or a Double's 17 and 3. }
function bruto_real_str(X: Double; Width, FracDigits: LongInt; IsSingle: Boolean): Pointer;
{ The QWord Q (held in an Int64) in decimal. }
function bruto_qword_str(Q: Int64): Pointer;
{ S right-aligned in Width characters (S itself if it is as long). }
function bruto_astr_pad(S: Pointer; Width: Int64): Pointer;
{ S left-aligned in Width characters. }
function bruto_astr_pad_right(S: Pointer; Width: Int64): Pointer;
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

type
  TBrutoIntfCount = function(Obj: Pointer): LongInt;

procedure bruto_intf_addref(Obj, ITable: Pointer);
var
  F: TBrutoIntfCount;
begin
  if Obj = nil then
    Exit;
  F := TBrutoIntfCount(PBrutoPointers(ITable)^[1]);
  F(Obj);
end;

procedure bruto_intf_release(Obj, ITable: Pointer);
var
  F: TBrutoIntfCount;
begin
  if Obj = nil then
    Exit;
  F := TBrutoIntfCount(PBrutoPointers(ITable)^[2]);
  F(Obj);
end;

function bruto_float_finite(X: Double): Boolean;
begin
  bruto_float_finite := X - X = 0;
end;

function bruto_float_error(A, B, R: Double; Op: LongInt): LongInt;
var
  Code, Bit: LongInt;
begin
  Code := 0;
  if R <> R then
  begin
    { NaN from numbers: an invalid operation. }
    if (A = A) and ((Op >= 5) or (B = B)) then
      Code := 207;
  end
  else if (Op = 4) and (B = 0) and bruto_float_finite(A) then
    Code := 208
  else if (Op = 6) and (A = 0) then
    Code := 208
  else if bruto_float_finite(A) and ((Op >= 5) or bruto_float_finite(B)) then
    Code := 205;
  case Code of
    207: Bit := 1;
    208: Bit := 4;
    205: Bit := 8;
  else
    Bit := 0;
  end;
  if ((BrutoFPUMaskFlip xor 50) and Bit) <> 0 then
    Code := 0;
  bruto_float_error := Code;
end;

type
  TBrutoIntfGetter = procedure(Obj, Out: Pointer);

function bruto_intf_query(Obj, Desc, Out: Pointer): Boolean;
var
  Table: Pointer;
  I, Count: Int64;
  Getter: TBrutoIntfGetter;
begin
  bruto_intf_query := False;
  if Obj = nil then
    Exit;
  { The table is the VMT's sixth word. }
  Table := PBrutoPointers(PBrutoPointers(Obj)^[0])^[5];
  if Table = nil then
    Exit;
  Count := PBrutoLength(Table)^;
  I := 0;
  while I < Count do
  begin
    if PBrutoPointers(Table)^[1 + I * 3] = Desc then
    begin
      if PBrutoPointers(Table)^[3 + I * 3] <> nil then
      begin
        Getter := TBrutoIntfGetter(PBrutoPointers(Table)^[3 + I * 3]);
        Getter(Obj, Out);
      end
      else
      begin
        PBrutoPointers(Out)^[0] := Obj;
        PBrutoPointers(Out)^[1] := PBrutoPointers(Table)^[2 + I * 3];
      end;
      bruto_intf_query := PBrutoPointers(Out)^[0] <> nil;
      Exit;
    end;
    I := I + 1;
  end;
end;

type
  TBrutoElemProc = procedure(P: Pointer; Count: Int64);

{ Free a dynamic array's block, without touching its elements. }
procedure bruto_dynarr_free_block(S: Pointer);
begin
  if (PBrutoMarker(S - 16)^ and BrutoAStrShifted) <> 0 then
    FreeMem(S - 48)
  else
    FreeMem(S - 32);
end;

procedure bruto_astr_free(S: Pointer);
var
  Cleanup: Pointer;
begin
  if (PBrutoMarker(S - 16)^ and BrutoDynArray) <> 0 then
  begin
    Cleanup := PPointer(S - 32)^;
    if Cleanup <> nil then
      TBrutoElemProc(Cleanup)(S, PBrutoLength(S - 8)^);
    bruto_dynarr_free_block(S);
  end
  else if (PBrutoMarker(S - 16)^ and BrutoAStrShifted) <> 0 then
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

{ Whether the variant at P holds a counted pointer. }
function bruto_var_counted(P: Pointer): Boolean;
var
  VType: Word;
begin
  VType := PWord(P)^;
  bruto_var_counted := (VType = 8) or (VType = $100) or (VType = $102)
    or ((VType and $2000) <> 0);
end;

procedure bruto_var_addref(P: Pointer);
begin
  if bruto_var_counted(P) then
    bruto_astr_addref(PPointer(P + 8)^);
end;

procedure bruto_var_release(P: Pointer);
begin
  if bruto_var_counted(P) then
    bruto_astr_release(PPointer(P + 8)^);
end;

procedure bruto_var_return(P: Pointer);
begin
  if bruto_var_counted(P) then
    bruto_astr_return(PPointer(P + 8)^);
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

function bruto_dynarr_block(Count, ElemSize: Int64; Cleanup, Addref: Pointer): Pointer;
var
  Block, Header: Pointer;
  Size: Int64;
begin
  Size := Count * ElemSize;
  GetMem(Block, 64 + Size);
  FillChar(Block^, 64 + Size, 0);
  Header := Block + 16;
  PBrutoMarker(Header)^ := BrutoAStrMarker or BrutoDynArray;
  if ((PtrUInt(Header) + 16) and 4095) < 16 then
  begin
    Header := Block + 32;
    PBrutoMarker(Header)^ := BrutoAStrMarker or BrutoDynArray or BrutoAStrShifted;
  end;
  PPointer(Header - 16)^ := Cleanup;
  PPointer(Header - 8)^ := Addref;
  PBrutoLength(Header + 8)^ := Count;
  if BrutoArc then
    bruto_pool_add(Header + 16);
  bruto_dynarr_block := Header + 16;
end;

function bruto_dynarr_owned(Count, ElemSize: Int64; Cleanup, Addref: Pointer): Pointer;
var
  P: Pointer;
begin
  P := bruto_dynarr_block(Count, ElemSize, Cleanup, Addref);
  PBrutoCount(P - 12)^ := 1;
  bruto_dynarr_owned := P;
end;

procedure bruto_dynarr_setlength(Slot: Pointer; NewCount, ElemSize: Int64; Cleanup, Addref: Pointer);
var
  P, Q: Pointer;
  Old, Keep: Int64;
begin
  P := PPointer(Slot)^;
  Old := bruto_astr_len(P);
  if NewCount <= 0 then
  begin
    if P <> nil then
      bruto_astr_release(P);
    PPointer(Slot)^ := nil;
    Exit;
  end;
  Q := bruto_dynarr_block(NewCount, ElemSize, Cleanup, Addref);
  Keep := Old;
  if NewCount < Keep then
    Keep := NewCount;
  if Keep > 0 then
    Move(P^, Q^, Keep * ElemSize);
  if P <> nil then
  begin
    if (PBrutoCount(P - 12)^ = 1) and ((PBrutoMarker(P - 16)^ and BrutoAStrPooled) = 0) then
    begin
      { The only reference: the kept elements moved; the others go. }
      if (Old > Keep) and (Cleanup <> nil) then
        TBrutoElemProc(Cleanup)(P + Keep * ElemSize, Old - Keep);
      bruto_dynarr_free_block(P);
    end
    else
    begin
      { Shared: the copies are references of their own. }
      if (Keep > 0) and (Addref <> nil) then
        TBrutoElemProc(Addref)(Q, Keep);
      bruto_astr_release(P);
    end;
  end;
  bruto_astr_addref(Q);
  PPointer(Slot)^ := Q;
end;

function bruto_dynarr_copy(P: Pointer; Start, Count, ElemSize: Int64): Pointer;
var
  Old: Int64;
  Q, Addref: Pointer;
begin
  bruto_dynarr_copy := nil;
  Old := bruto_astr_len(P);
  if Start < 0 then
    Start := 0;
  if Start > Old then
    Start := Old;
  if Count > Old - Start then
    Count := Old - Start;
  if Count <= 0 then
    Exit;
  Addref := PPointer(P - 24)^;
  Q := bruto_dynarr_block(Count, ElemSize, PPointer(P - 32)^, Addref);
  Move((P + Start * ElemSize)^, Q^, Count * ElemSize);
  if Addref <> nil then
    TBrutoElemProc(Addref)(Q, Count);
  bruto_dynarr_copy := Q;
end;

procedure bruto_dynarr_delete(Slot: Pointer; Index, Count, ElemSize: Int64);
var
  P, Cleanup, Addref: Pointer;
  Len: Int64;
begin
  P := PPointer(Slot)^;
  Len := bruto_astr_len(P);
  if (P = nil) or (Index < 0) or (Index >= Len) or (Count <= 0) then
    Exit;
  if Count > Len - Index then
    Count := Len - Index;
  Cleanup := PPointer(P - 32)^;
  Addref := PPointer(P - 24)^;
  { A block of its own, then the elements go and the rest move down. }
  bruto_dynarr_setlength(Slot, Len, ElemSize, Cleanup, Addref);
  P := PPointer(Slot)^;
  if Cleanup <> nil then
    TBrutoElemProc(Cleanup)(P + Index * ElemSize, Count);
  if Len - Index - Count > 0 then
    Move((P + (Index + Count) * ElemSize)^, (P + Index * ElemSize)^, (Len - Index - Count) * ElemSize);
  FillChar((P + (Len - Count) * ElemSize)^, Count * ElemSize, 0);
  bruto_dynarr_setlength(Slot, Len - Count, ElemSize, Cleanup, Addref);
end;

function bruto_dynarr_insert_gap(Slot: Pointer; Index, ElemSize: Int64; Cleanup, Addref: Pointer): Int64;
var
  P: Pointer;
  Len: Int64;
begin
  P := PPointer(Slot)^;
  Len := bruto_astr_len(P);
  if Index < 0 then
    Index := 0;
  if Index > Len then
    Index := Len;
  bruto_dynarr_setlength(Slot, Len + 1, ElemSize, Cleanup, Addref);
  P := PPointer(Slot)^;
  if Len - Index > 0 then
    Move((P + Index * ElemSize)^, (P + (Index + 1) * ElemSize)^, (Len - Index) * ElemSize);
  FillChar((P + Index * ElemSize)^, ElemSize, 0);
  bruto_dynarr_insert_gap := Index;
end;

function bruto_rt_snprintf(Buf: PChar; Size: PtrUInt; Fmt: PChar): LongInt; cdecl; varargs;
  external 'c' name 'snprintf';
function bruto_rt_fflush(Stream: Pointer): LongInt; cdecl; external 'c' name 'fflush';
procedure bruto_rt_exit(Code: LongInt); cdecl; external 'c' name 'exit';
function bruto_rt_signal(Sig: LongInt; Handler: Pointer): Pointer; cdecl; external 'c' name 'signal';
{$ifdef WINDOWS}
function bruto_rt_write(Fd: LongInt; Buf: PChar; Count: LongWord): LongInt; cdecl; external 'c' name '_write';
{$else}
function bruto_rt_write(Fd: LongInt; Buf: PChar; Count: PtrUInt): PtrInt; cdecl; external 'c' name 'write';
function bruto_rt_sigemptyset(Set_: Pointer): LongInt; cdecl; external 'c' name 'sigemptyset';
function bruto_rt_sigaddset(Set_: Pointer; Sig: LongInt): LongInt; cdecl; external 'c' name 'sigaddset';
function bruto_rt_sigprocmask(How: LongInt; Set_, Old: Pointer): LongInt; cdecl; external 'c' name 'sigprocmask';
{$endif}

procedure bruto_runtime_error(Code: LongInt; Address: Pointer; UseHook: Boolean);
var
  Buf: array[0..127] of Char;
  N: LongInt;
begin
  if UseHook and (BrutoRunErrorHook <> nil) then
    BrutoRunErrorHook(Code);
  bruto_rt_fflush(nil);
  N := bruto_rt_snprintf(@Buf, SizeOf(Buf), 'Runtime error %d at $%016llX'#10'  $%016llX'#10#10,
    Code, PtrUInt(Address), PtrUInt(Address));
  bruto_rt_write(2, @Buf, N);
  bruto_rt_exit(Code);
end;

function bruto_qword_str(Q: Int64): Pointer;
var
  Buf: array[0..31] of Char;
  N: LongInt;
  R: Pointer;
begin
  N := bruto_rt_snprintf(@Buf, SizeOf(Buf), '%llu', Q);
  R := bruto_astr_alloc(N);
  Move(Buf, R^, N);
  bruto_qword_str := R;
end;

function bruto_astr_pad(S: Pointer; Width: Int64): Pointer;
var
  Len, I: Int64;
  R: PBrutoBytes;
begin
  Len := bruto_astr_len(S);
  if Len >= Width then
  begin
    bruto_astr_pad := S;
    Exit;
  end;
  R := PBrutoBytes(bruto_astr_alloc(Width));
  for I := 0 to Width - Len - 1 do
    R^[I] := 32;
  if Len > 0 then
    Move(S^, R^[Width - Len], Len);
  bruto_astr_pad := R;
end;

function bruto_astr_pad_right(S: Pointer; Width: Int64): Pointer;
var
  Len, I: Int64;
  R: PBrutoBytes;
begin
  Len := bruto_astr_len(S);
  if Len >= Width then
  begin
    bruto_astr_pad_right := S;
    Exit;
  end;
  R := PBrutoBytes(bruto_astr_alloc(Width));
  if Len > 0 then
    Move(S^, R^[0], Len);
  for I := Len to Width - 1 do
    R^[I] := 32;
  bruto_astr_pad_right := R;
end;

{ ── Reals as text, as FPC's str_real (flt_core.inc) makes them ── }

type
  TBrutoDigits = array[0..47] of Byte;
  TBrutoRealText = array[0..299] of Char;

{ Round the NCur digits in Buf to NMax: half to even, or (Even false) half
  up, also rounding ...4999..98 up as FPC does. The result is 1 when the
  first digit carried out (the decimal point moves). }
function bruto_round_digits(var Buf: TBrutoDigits; var NCur: LongInt; NMax: LongInt;
  Even: Boolean): LongInt;
var
  N, I: LongInt;
  Rd, Sticky: Byte;
begin
  bruto_round_digits := 0;
  N := NCur;
  NCur := NMax;
  Rd := Buf[NMax];
  if (not Even) and (Rd = 4) and (NMax < N - 3) and (Buf[N - 2] >= 8) then
  begin
    I := N - 2;
    repeat
      I := I - 1;
    until (I = NMax) or (Buf[I] <> 9);
    if I = NMax then
      Rd := 9;
  end;
  if Rd < 5 then
    Exit;
  if (Rd = 5) and Even and ((NMax = 0) or ((Buf[NMax - 1] and 1) = 0)) then
  begin
    Sticky := 0;
    while (N > NMax + 1) and (Sticky = 0) do
    begin
      N := N - 1;
      Sticky := Buf[N];
    end;
    if Sticky = 0 then
      Exit;
  end;
  while NMax > 0 do
  begin
    NMax := NMax - 1;
    Buf[NMax] := Buf[NMax] + 1;
    if Buf[NMax] < 10 then
    begin
      NCur := NMax + 1;
      Exit;
    end;
  end;
  Buf[0] := 1;
  NCur := 1;
  bruto_round_digits := 1;
end;

procedure bruto_text_add(var T: TBrutoRealText; var Len: LongInt; C: Char; Count: LongInt);
begin
  while (Count > 0) and (Len < 299) do
  begin
    T[Len] := C;
    Len := Len + 1;
    Count := Count - 1;
  end;
end;

{ The fixed point form; False if it would pass 255 characters. }
function bruto_real_fixed(var T: TBrutoRealText; var Len: LongInt; Minus: Boolean;
  const Digits: TBrutoDigits; NHave, DotPos, MinWidth, FracDigits: LongInt): Boolean;
var
  R: TBrutoDigits;
  CutAt, NBefore, NBefore0, NAfter0, NAfter, NTail0, Total, Spaces, J: LongInt;
begin
  bruto_real_fixed := False;
  R := Digits;
  CutAt := DotPos + FracDigits;
  if CutAt < 0 then
    NHave := 0
  else if CutAt < NHave then
    DotPos := DotPos + bruto_round_digits(R, NHave, CutAt, False);
  if (DotPos <= 0) or (NHave = 0) then
  begin
    NBefore := 0;
    NBefore0 := 1;
  end
  else if DotPos > NHave then
  begin
    NBefore := NHave;
    NBefore0 := DotPos - NHave;
  end
  else
  begin
    NBefore := DotPos;
    NBefore0 := 0;
  end;
  if DotPos < 0 then
    NAfter0 := -DotPos
  else
    NAfter0 := 0;
  if NAfter0 > FracDigits then
    NAfter0 := FracDigits;
  NAfter := NHave - NBefore;
  NTail0 := FracDigits - NAfter - NAfter0;
  Total := Ord(Minus) + NBefore + NBefore0;
  if FracDigits > 0 then
    Total := Total + NAfter0 + NAfter + NTail0 + 1;
  if Total > 255 then
    Exit;
  Spaces := MinWidth - Total;
  if Spaces > 255 - Total then
    Spaces := 255 - Total;
  Len := 0;
  bruto_text_add(T, Len, ' ', Spaces);
  if Minus then
    bruto_text_add(T, Len, '-', 1);
  J := 0;
  while NBefore > 0 do
  begin
    bruto_text_add(T, Len, Chr(R[J] + 48), 1);
    J := J + 1;
    NBefore := NBefore - 1;
  end;
  bruto_text_add(T, Len, '0', NBefore0);
  if FracDigits <> 0 then
  begin
    bruto_text_add(T, Len, '.', 1);
    bruto_text_add(T, Len, '0', NAfter0);
    while NAfter > 0 do
    begin
      bruto_text_add(T, Len, Chr(R[J] + 48), 1);
      J := J + 1;
      NAfter := NAfter - 1;
    end;
    bruto_text_add(T, Len, '0', NTail0);
  end;
  bruto_real_fixed := True;
end;

procedure bruto_real_exponential(var T: TBrutoRealText; var Len: LongInt; Minus: Boolean;
  const Digits: TBrutoDigits; NHave, NReq, DExp, NDigExp, MinWidth: LongInt);
var
  EMinus: Boolean;
  ExpText: array[0..15] of Char;
  NExp, Total, Spaces, J: LongInt;
begin
  EMinus := DExp < 0;
  if EMinus then
    DExp := -DExp;
  NExp := 0;
  repeat
    ExpText[NExp] := Chr(DExp mod 10 + 48);
    DExp := DExp div 10;
    NExp := NExp + 1;
  until DExp = 0;
  if NExp <= NDigExp then
    Total := NDigExp
  else
    Total := NExp;
  Total := Total + 1 + NReq + 2;
  if NReq > 1 then
    Total := Total + 1;
  Spaces := MinWidth - Total;
  if Spaces > 255 - Total then
    Spaces := 255 - Total;
  Len := 0;
  bruto_text_add(T, Len, ' ', Spaces);
  if Minus then
    bruto_text_add(T, Len, '-', 1)
  else
    bruto_text_add(T, Len, ' ', 1);
  if NHave > 0 then
    bruto_text_add(T, Len, Chr(Digits[0] + 48), 1)
  else
    bruto_text_add(T, Len, '0', 1);
  if NReq > 1 then
    bruto_text_add(T, Len, '.', 1);
  J := 1;
  while (J < NHave) and (J < NReq) do
  begin
    bruto_text_add(T, Len, Chr(Digits[J] + 48), 1);
    J := J + 1;
  end;
  bruto_text_add(T, Len, '0', NReq - J);
  bruto_text_add(T, Len, 'E', 1);
  if EMinus then
    bruto_text_add(T, Len, '-', 1)
  else
    bruto_text_add(T, Len, '+', 1);
  bruto_text_add(T, Len, '0', NDigExp - NExp);
  while NExp > 0 do
  begin
    NExp := NExp - 1;
    bruto_text_add(T, Len, ExpText[NExp], 1);
  end;
end;

{ ── FPC's Grisu1 digit generation (flt_core.inc, 64-bit), so the digits
  are FPC's to the last one. Unsigned 64-bit values are kept in Int64. }

type
  TBrutoDiyFp = record
    F: Int64;
    E: LongInt;
  end;
  TBrutoPow10 = record
    F: Int64;
    E: LongInt;
    E10: LongInt;
  end;

function bruto_ult(A, B: Int64): Boolean;
begin
  bruto_ult := (A xor (Int64(1) shl 63)) < (B xor (Int64(1) shl 63));
end;

{ X div D, X unsigned, 0 < D < 2^62. }
function bruto_udiv(X, D: Int64): Int64;
var
  Q, R: Int64;
begin
  if X >= 0 then
  begin
    bruto_udiv := X div D;
    Exit;
  end;
  Q := ((X shr 1) div D) shl 1;
  R := X - Q * D;
  while not bruto_ult(R, D) do
  begin
    Q := Q + 1;
    R := R - D;
  end;
  bruto_udiv := Q;
end;

function bruto_diy_multiply(const X: TBrutoDiyFp; YF: Int64; YE: LongInt;
  Normalize: Boolean): TBrutoDiyFp;
var
  A, B, C, D, AC, BC, AD, BD, T1: Int64;
  R: TBrutoDiyFp;
begin
  A := X.F shr 32;
  B := X.F and $FFFFFFFF;
  C := YF shr 32;
  D := YF and $FFFFFFFF;
  AC := A * C;
  BC := B * C;
  AD := A * D;
  BD := B * D;
  T1 := (Int64(1) shl 31) + (BD shr 32) + (BC and $FFFFFFFF) + (AD and $FFFFFFFF);
  R.F := AC + (AD shr 32) + (BC shr 32) + (T1 shr 32);
  R.E := X.E + YE + 64;
  if Normalize and ((R.F shr 63) = 0) then
  begin
    R.F := R.F + R.F;
    R.E := R.E - 1;
  end;
  bruto_diy_multiply := R;
end;

procedure bruto_pow10_entry(var P: TBrutoPow10; Index: LongInt);
begin
  case Index of
    { base }
    0: begin P.F := Int64($825ECC24C8737830); P.E := -362; P.E10 := -90; end;
    1: begin P.F := Int64($E2280B6C20DD5232); P.E := -303; P.E10 := -72; end;
    2: begin P.F := Int64($C428D05AA4751E4D); P.E := -243; P.E10 := -54; end;
    3: begin P.F := Int64($AA242499697392D3); P.E := -183; P.E10 := -36; end;
    4: begin P.F := Int64($9392EE8E921D5D07); P.E := -123; P.E10 := -18; end;
    5: begin P.F := Int64($8000000000000000); P.E := -63; P.E10 := 0; end;
    6: begin P.F := Int64($DE0B6B3A76400000); P.E := -4; P.E10 := 18; end;
    7: begin P.F := Int64($C097CE7BC90715B3); P.E := 56; P.E10 := 36; end;
    8: begin P.F := Int64($A70C3C40A64E6C52); P.E := 116; P.E10 := 54; end;
    9: begin P.F := Int64($90E40FBEEA1D3A4B); P.E := 176; P.E10 := 72; end;
    { factor_plus }
    10: begin P.F := Int64($F6C69A72A3989F5C); P.E := 534; P.E10 := 180; end;
    11: begin P.F := Int64($EDE24AE798EC8284); P.E := 1132; P.E10 := 360; end;
    { factor_minus }
    12: begin P.F := Int64($84C8D4DFD2C63F3B); P.E := -661; P.E10 := -180; end;
    13: begin P.F := Int64($89BF722840327F82); P.E := -1259; P.E10 := -360; end;
  end;
end;

function bruto_pow10_corrector(I: LongInt): LongInt;
begin
  case I of
    4, 8, 11, 12, 13, 17, 44: bruto_pow10_corrector := 1;
    9, 14, 19, 30, 33, 39, 48: bruto_pow10_corrector := -1;
  else
    bruto_pow10_corrector := 0;
  end;
end;

procedure bruto_cached_power10(Exp10: LongInt; var Factor: TBrutoPow10);
var
  I, XMul, INod, Min10: LongInt;
  A, M: TBrutoPow10;
  X, R: TBrutoDiyFp;
begin
  Min10 := -90 + -360;
  if Exp10 <= Min10 then
    I := 0
  else
  begin
    I := (Exp10 - Min10) div 18;
    if I * 18 + Min10 <> Exp10 then
      I := I + 1;
    if I > 49 then
      I := 49;
  end;
  INod := I mod 10;
  XMul := (I div 10) - 2;
  bruto_pow10_entry(A, INod);
  if XMul = 0 then
  begin
    Factor := A;
    Exit;
  end;
  if XMul > 0 then
    bruto_pow10_entry(M, 10 + XMul - 1)
  else
    bruto_pow10_entry(M, 12 - (XMul + 1));
  Factor.E10 := A.E10 + M.E10;
  if A.E10 = 0 then
  begin
    Factor.F := M.F;
    Factor.E := M.E;
    Exit;
  end;
  X.F := A.F;
  X.E := A.E;
  R := bruto_diy_multiply(X, M.F, M.E, True);
  Factor.F := R.F + bruto_pow10_corrector(I);
  Factor.E := R.E;
end;

function bruto_gen_digits_32(var Buf: TBrutoDigits; Pos: LongInt; X: Int64; Pad9: Boolean): LongInt;
var
  N: LongInt;
  M, Z: Int64;
begin
  N := 0;
  M := X;
  while M <> 0 do
  begin
    N := N + 1;
    M := M div 10;
  end;
  if Pad9 and (N < 9) then
    N := 9;
  bruto_gen_digits_32 := N;
  M := X;
  while N > 0 do
  begin
    N := N - 1;
    Z := M div 10;
    Buf[Pos + N] := M - Z * 10;
    M := Z;
  end;
end;

function bruto_gen_digits_64(var Buf: TBrutoDigits; Pos: LongInt; X: Int64): LongInt;
var
  N: LongInt;
  Temp, SplitL, SplitM, SplitH: Int64;
begin
  if bruto_ult(X, 1000000000) then
  begin
    SplitH := 0;
    SplitM := 0;
    SplitL := X;
  end
  else
  begin
    Temp := bruto_udiv(X, 1000000000);
    SplitL := X - Temp * 1000000000;
    if Temp < 1000000000 then
    begin
      SplitH := 0;
      SplitM := Temp;
    end
    else
    begin
      SplitH := Temp div 1000000000;
      SplitM := Temp - SplitH * 1000000000;
    end;
  end;
  N := bruto_gen_digits_32(Buf, Pos, SplitH, False);
  N := N + bruto_gen_digits_32(Buf, Pos + N, SplitM, N <> 0);
  N := N + bruto_gen_digits_32(Buf, Pos + N, SplitL, N <> 0);
  bruto_gen_digits_64 := N;
end;

{ The decimal digits of the non-zero, finite X (as FPC's str_real makes
  them): NeedDig digits rounded half to even, at most; DotPos is where the
  decimal point goes. }
procedure bruto_real_digits(X: Double; NeedDig: LongInt; var Digits: TBrutoDigits;
  var N, DotPos: LongInt);
var
  Bits, OneMask, Fl: Int64;
  W, D: TBrutoDiyFp;
  CMk: TBrutoPow10;
  Shift, Mk, OneE, K: LongInt;
  DExp: Double;
  Sticky: Boolean;
begin
  Bits := PBrutoLength(@X)^;
  W.E := (Bits shr 52) and $7FF;
  W.F := Bits and $000FFFFFFFFFFFFF;
  if W.E <> 0 then
  begin
    W.F := W.F or (Int64(1) shl 52);
    Shift := 11;
  end
  else
  begin
    { A denormal: normalize it. }
    Shift := 0;
    while ((W.F shl Shift) shr 63) = 0 do
      Shift := Shift + 1;
    W.E := W.E + 1;
  end;
  W.F := W.F shl Shift;
  W.E := W.E - (1023 + Shift + 52);
  if (-61 <= W.E) and (W.E <= 0) then
  begin
    D := W;
    CMk.E10 := 0;
  end
  else
  begin
    { k_comp: ceiling((alpha - e) * log10(2)) }
    K := -61 - W.E;
    DExp := K * 0.301029995663981195213738894724493027;
    Mk := Trunc(DExp);
    if (K > 0) and (DExp <> Mk) then
      Mk := Mk + 1;
    bruto_cached_power10(Mk, CMk);
    if CMk.E10 = 0 then
      D := W
    else
      D := bruto_diy_multiply(W, CMk.F, CMk.E, False);
  end;
  N := bruto_gen_digits_64(Digits, 0, D.F shr (-D.E));
  DotPos := N;
  Sticky := False;
  if D.E < 0 then
  begin
    OneE := D.E;
    OneMask := (Int64(1) shl (-D.E)) - 1;
    Fl := D.F and OneMask;
    while (OneE < -29) and (N < NeedDig + 1) and (Fl <> 0) do
    begin
      Fl := Fl + (Fl shl 2);
      OneMask := OneMask shr 1;
      OneE := OneE + 1;
      Digits[N] := Fl shr (-OneE);
      Fl := Fl and OneMask;
      N := N + 1;
    end;
    if N >= NeedDig + 1 then
      Sticky := Fl <> 0
    else
    begin
      while (N < NeedDig + 1) and (Fl <> 0) do
      begin
        Fl := Fl + (Fl shl 2);
        OneMask := OneMask shr 1;
        OneE := OneE + 1;
        Digits[N] := Fl shr (-OneE);
        Fl := Fl and OneMask;
        N := N + 1;
      end;
      Sticky := Fl <> 0;
    end;
  end;
  if Sticky and (N >= NeedDig + 1) then
  begin
    N := NeedDig + 2;
    Digits[NeedDig + 1] := 1;
  end;
  if N > NeedDig then
    DotPos := DotPos + bruto_round_digits(Digits, N, NeedDig, True);
  DotPos := DotPos - CMk.E10;
end;

function bruto_real_str(X: Double; Width, FracDigits: LongInt; IsSingle: Boolean): Pointer;
var
  T: TBrutoRealText;
  Spec: array[0..3] of Char;
  Digits: TBrutoDigits;
  Len, NeedDig, ExpDig, SciDig, MinWidth, N, I, DotPos, SpecLen: LongInt;
  Minus: Boolean;
  R: Pointer;
begin
  if FracDigits > 216 then
    FracDigits := 216;
  if Width > 255 then
    Width := 255;
  MinWidth := Width;
  if IsSingle then
  begin
    NeedDig := 10;
    ExpDig := 2;
  end
  else
  begin
    NeedDig := 17;
    ExpDig := 3;
  end;
  if MinWidth < 0 then
    SciDig := NeedDig
  else
  begin
    SciDig := MinWidth - 4 - ExpDig;
    if SciDig < 2 then
      SciDig := 2;
    if SciDig > NeedDig then
      SciDig := NeedDig;
  end;
  Minus := PBrutoLength(@X)^ < 0;
  Len := 0;
  if X - X <> 0 then
  begin
    { Infinity (signed) or NaN (signless). }
    if MinWidth < 0 then
      MinWidth := NeedDig + ExpDig + 4;
    if X = X then
    begin
      if Minus then
        Spec[0] := '-'
      else
        Spec[0] := '+';
      Spec[1] := 'I';
      Spec[2] := 'n';
      Spec[3] := 'f';
      SpecLen := 4;
    end
    else
    begin
      Spec[0] := 'N';
      Spec[1] := 'a';
      Spec[2] := 'n';
      SpecLen := 3;
    end;
    bruto_text_add(T, Len, ' ', MinWidth - SpecLen);
    for I := 0 to SpecLen - 1 do
      bruto_text_add(T, Len, Spec[I], 1);
  end
  else if X = 0 then
  begin
    if (FracDigits < 0) or not bruto_real_fixed(T, Len, Minus, Digits, 0, 1, MinWidth, FracDigits) then
      bruto_real_exponential(T, Len, Minus, Digits, 0, SciDig, 0, ExpDig, MinWidth);
  end
  else
  begin
    bruto_real_digits(X, NeedDig, Digits, N, DotPos);
    if (FracDigits < 0) or not bruto_real_fixed(T, Len, Minus, Digits, N, DotPos, MinWidth, FracDigits) then
    begin
      if N > SciDig then
        DotPos := DotPos + bruto_round_digits(Digits, N, SciDig, False);
      bruto_real_exponential(T, Len, Minus, Digits, N, SciDig, DotPos - 1, ExpDig, MinWidth);
    end;
  end;
  R := bruto_astr_alloc(Len);
  if Len > 0 then
    Move(T, R^, Len);
  bruto_real_str := R;
end;

{ SIGSEGV / SIGBUS. The signal is unblocked first: a handler raising an
  exception doesn't return through the kernel. }
procedure bruto_fault_handler(Sig: LongInt);
{$ifndef WINDOWS}
var
  Mask: array[0..127] of Byte;
{$endif}
begin
{$ifndef WINDOWS}
  bruto_rt_sigemptyset(@Mask);
  bruto_rt_sigaddset(@Mask, Sig);
  { SIG_UNBLOCK: 2 on macOS, 1 on Linux. }
{$ifdef DARWIN}
  bruto_rt_sigprocmask(2, @Mask, nil);
{$else}
  bruto_rt_sigprocmask(1, @Mask, nil);
{$endif}
{$endif}
  bruto_runtime_error(216, nil, True);
end;

procedure bruto_install_fault_handlers;
begin
  bruto_rt_signal(11, @bruto_fault_handler);
{$ifdef DARWIN}
  bruto_rt_signal(10, @bruto_fault_handler);
{$else}
{$ifndef WINDOWS}
  bruto_rt_signal(7, @bruto_fault_handler);
{$endif}
{$endif}
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
