{ Bruto Pascal runtime: UnicodeString. Code generation calls these
  routines for UnicodeString and WideChar operations (and Val); they are
  added to every program, with FPC's UTF-8 conversion routines.

  A UnicodeString is a pointer to UTF-16 code units, followed by a zero
  unit and preceded by the length (an Int64), as in FPC. Nil is the
  empty string. Bruto has no reference counts, so a string is never
  changed in place: every operation that changes one builds a copy.
  String (AnsiString) is UTF-8, as FPC's default code page on Unix. }
unit BrutoUStrings;

interface

type
  BrutoUnits = array[0..$FFFFFFF] of Word;
  PBrutoUnits = ^BrutoUnits;
  BrutoBytes = array[0..$FFFFFFF] of Byte;
  PBrutoBytes = ^BrutoBytes;
  PBrutoLength = ^Int64;

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

implementation

function bruto_towupper(C: LongInt): LongInt; cdecl; external 'c' name 'towupper';
function bruto_towlower(C: LongInt): LongInt; cdecl; external 'c' name 'towlower';
function bruto_strtod(S: Pointer; EndPtr: Pointer): Double; cdecl; external 'c' name 'strtod';

function bruto_ustr_length(S: Pointer): Int64;
begin
  if S = nil then
    bruto_ustr_length := 0
  else
    bruto_ustr_length := PBrutoLength(S - 8)^;
end;

{ A string of Len units, all zero, or nil when Len is 0. }
function bruto_ustr_alloc(Len: Int64): Pointer;
var
  Block: PBrutoBytes;
begin
  if Len <= 0 then
  begin
    bruto_ustr_alloc := nil;
    Exit;
  end;
  GetMem(Block, 8 + (Len + 1) * 2);
  FillChar(Block^, 8 + (Len + 1) * 2, 0);
  PBrutoLength(Block)^ := Len;
  bruto_ustr_alloc := Pointer(Block) + 8;
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
  I, Len, Code: Int64;
  R: PBrutoUnits;
begin
  bruto_utf8_to_ustr := nil;
  if S = nil then
    Exit;
  Bytes := PBrutoBytes(S);
  Len := 0;
  I := 0;
  while Bytes^[I] <> 0 do
  begin
    I := I + bruto_utf8_decode(Bytes, I, Code);
    Len := Len + bruto_utf16_units(Code);
  end;
  if Len = 0 then
    Exit;
  R := PBrutoUnits(bruto_ustr_alloc(Len));
  Len := 0;
  I := 0;
  while Bytes^[I] <> 0 do
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
  GetMem(R, Size + 1);
  I := 0;
  J := 0;
  while I < Len do
  begin
    Code := bruto_utf16_code(U, I, Len, Units);
    J := J + bruto_utf8_encode(R, J, Code);
    I := I + Units;
  end;
  R^[J] := 0;
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
  GetMem(R, 5);
  R^[bruto_utf8_encode(R, 0, C)] := 0;
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

end.
