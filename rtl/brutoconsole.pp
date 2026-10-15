{ Bruto Pascal runtime: the terminal underneath the Video, Keyboard and
  Mouse units. It switches the terminal to raw mode, buffers output,
  and decodes input (keys, ANSI escape sequences and SGR mouse reports)
  into DOS-style key codes and mouse events, which is what Free Vision
  expects. }
unit BrutoConsole;

interface

uses UnixType, BaseUnix;

type
  { A key: Code is the DOS scan code in the high byte and the character
    in the low one; Shift holds kbLeftShift/kbCtrl/kbAlt-style bits. }
  TConsoleKey = record
    Code: Word;
    Shift: Byte;
  end;

  TConsoleMouse = record
    Buttons, X, Y, Action: Word;
  end;

  TConsoleProc = procedure;

const
  ConsoleShift = 1;
  ConsoleCtrl = 4;
  ConsoleAlt = 8;
  ConsoleMouseDown = 1;
  ConsoleMouseUp = 2;
  ConsoleMouseMove = 4;

  { The Unicode character each code page 437 byte stands for. }
  CP437: array[0..255] of Word = (
    $0020, $263A, $263B, $2665, $2666, $2663, $2660, $2022,
    $25D8, $25CB, $25D9, $2642, $2640, $266A, $266B, $263C,
    $25BA, $25C4, $2195, $203C, $00B6, $00A7, $25AC, $21A8,
    $2191, $2193, $2192, $2190, $221F, $2194, $25B2, $25BC,
    $0020, $0021, $0022, $0023, $0024, $0025, $0026, $0027,
    $0028, $0029, $002A, $002B, $002C, $002D, $002E, $002F,
    $0030, $0031, $0032, $0033, $0034, $0035, $0036, $0037,
    $0038, $0039, $003A, $003B, $003C, $003D, $003E, $003F,
    $0040, $0041, $0042, $0043, $0044, $0045, $0046, $0047,
    $0048, $0049, $004A, $004B, $004C, $004D, $004E, $004F,
    $0050, $0051, $0052, $0053, $0054, $0055, $0056, $0057,
    $0058, $0059, $005A, $005B, $005C, $005D, $005E, $005F,
    $0060, $0061, $0062, $0063, $0064, $0065, $0066, $0067,
    $0068, $0069, $006A, $006B, $006C, $006D, $006E, $006F,
    $0070, $0071, $0072, $0073, $0074, $0075, $0076, $0077,
    $0078, $0079, $007A, $007B, $007C, $007D, $007E, $2302,
    $00C7, $00FC, $00E9, $00E2, $00E4, $00E0, $00E5, $00E7,
    $00EA, $00EB, $00E8, $00EF, $00EE, $00EC, $00C4, $00C5,
    $00C9, $00E6, $00C6, $00F4, $00F6, $00F2, $00FB, $00F9,
    $00FF, $00D6, $00DC, $00A2, $00A3, $00A5, $20A7, $0192,
    $00E1, $00ED, $00F3, $00FA, $00F1, $00D1, $00AA, $00BA,
    $00BF, $2310, $00AC, $00BD, $00BC, $00A1, $00AB, $00BB,
    $2591, $2592, $2593, $2502, $2524, $2561, $2562, $2556,
    $2555, $2563, $2551, $2557, $255D, $255C, $255B, $2510,
    $2514, $2534, $252C, $251C, $2500, $253C, $255E, $255F,
    $255A, $2554, $2569, $2566, $2560, $2550, $256C, $2567,
    $2568, $2564, $2565, $2559, $2558, $2552, $2553, $256B,
    $256A, $2518, $250C, $2588, $2584, $258C, $2590, $2580,
    $03B1, $00DF, $0393, $03C0, $03A3, $03C3, $00B5, $03C4,
    $03A6, $0398, $03A9, $03B4, $221E, $03C6, $03B5, $2229,
    $2261, $00B1, $2265, $2264, $2320, $2321, $00F7, $2248,
    $00B0, $2219, $00B7, $221A, $207F, $00B2, $25A0, $00A0
  );

var
  ConsoleMouseX, ConsoleMouseY, ConsoleMouseButtons: Word;

procedure ConsoleRawMode;
procedure ConsoleRestoreMode;
procedure ConsoleSize(var Cols, Rows: Word);

procedure ConsoleOutChar(C: Char);
procedure ConsoleOutStr(const S: string);
procedure ConsoleOutNum(N: LongInt);
{ Write code page 437 character Ch as UTF-8. }
procedure ConsoleOutCP437(Ch: Byte);
procedure ConsoleFlush;

{ The next key, without removing it; False if there is none. }
function ConsolePeekKey(var K: TConsoleKey): Boolean;
{ Remove and return the next key, waiting for one. }
procedure ConsoleGetKey(var K: TConsoleKey);
function ConsolePeekMouse(var M: TConsoleMouse): Boolean;
procedure ConsoleGetMouse(var M: TConsoleMouse);
procedure ConsolePutMouse(const M: TConsoleMouse);

{ Call Proc when input starts with Seq, instead of decoding it as keys;
  Proc can read what follows with ConsoleReadRaw. }
procedure ConsoleAddSequence(const Seq: string; Proc: TConsoleProc);
{ Whether undecoded input is waiting. }
function ConsoleKeyPressed: Boolean;
{ The next undecoded input byte, waiting for one. }
function ConsoleReadRaw: Char;

implementation

type
  TPollFd = record
    Fd: cint;
    Events, REvents: cshort;
  end;
  TWinSize = record
    Rows, Cols, XPixel, YPixel: Word;
  end;
  TTermios = array[0..255] of Byte;

function c_tcgetattr(Fd: cint; T: Pointer): cint; cdecl; external 'c' name 'tcgetattr';
function c_tcsetattr(Fd: cint; Action: cint; T: Pointer): cint; cdecl; external 'c' name 'tcsetattr';
procedure c_cfmakeraw(T: Pointer); cdecl; external 'c' name 'cfmakeraw';
function c_poll(Fds: Pointer; NFds: culong; Timeout: cint): cint; cdecl; external 'c' name 'poll';
function c_atexit(Fn: Pointer): cint; cdecl; external 'c' name 'atexit';

const
  KeyQueueSize = 64;
  MouseQueueSize = 64;
{$ifdef DARWIN}
  TIOCGWINSZ = $40087468;
{$else}
  TIOCGWINSZ = $5413;
{$endif}

var
  SavedMode, RawMode: TTermios;
  InRaw: Boolean;
  ExitHooked: Boolean;

  OutBuf: array[0..16383] of Char;
  OutLen: LongInt;

  InBuf: array[0..1023] of Byte;
  InLen: LongInt;

  Keys: array[0..KeyQueueSize - 1] of TConsoleKey;
  KeyHead, KeyCount: LongInt;
  Mice: array[0..MouseQueueSize - 1] of TConsoleMouse;
  MouseHead, MouseCount: LongInt;

  SpecialSeqs: array[0..15] of string;
  SpecialProcs: array[0..15] of TConsoleProc;
  SpecialCount: LongInt;

{ ── terminal mode ───────────────────────────────────────── }

procedure RestoreAtExit; cdecl;
begin
  ConsoleRestoreMode;
end;

procedure ConsoleRawMode;
begin
  if InRaw then
    Exit;
  if c_tcgetattr(0, @SavedMode) <> 0 then
    Exit;
  RawMode := SavedMode;
  c_cfmakeraw(@RawMode);
  c_tcsetattr(0, 0, @RawMode);
  InRaw := True;
  if not ExitHooked then
  begin
    c_atexit(@RestoreAtExit);
    ExitHooked := True;
  end;
end;

procedure ConsoleRestoreMode;
begin
  ConsoleFlush;
  if InRaw then
  begin
    c_tcsetattr(0, 0, @SavedMode);
    InRaw := False;
  end;
end;

procedure ConsoleSize(var Cols, Rows: Word);
var
  Ws: TWinSize;
begin
  Ws.Cols := 0;
  Ws.Rows := 0;
  if (FpIOCtl(1, TIOCGWINSZ, @Ws) = 0) and (Ws.Cols > 0) and (Ws.Rows > 0) then
  begin
    Cols := Ws.Cols;
    Rows := Ws.Rows;
  end
  else
  begin
    Cols := 80;
    Rows := 25;
  end;
end;

{ ── output ──────────────────────────────────────────────── }

procedure ConsoleFlush;
var
  Done, N: Int64;
begin
  Done := 0;
  while Done < OutLen do
  begin
    N := FpWrite(1, @OutBuf[Done], OutLen - Done);
    if N <= 0 then
      Break;
    Done := Done + N;
  end;
  OutLen := 0;
end;

procedure ConsoleOutChar(C: Char);
begin
  if OutLen >= 16384 then
    ConsoleFlush;
  OutBuf[OutLen] := C;
  OutLen := OutLen + 1;
end;

procedure ConsoleOutStr(const S: string);
var
  I: LongInt;
begin
  for I := 1 to Length(S) do
    ConsoleOutChar(S[I]);
end;

procedure ConsoleOutNum(N: LongInt);
begin
  if N >= 10 then
    ConsoleOutNum(N div 10);
  ConsoleOutChar(Chr(Ord('0') + N mod 10));
end;

procedure ConsoleOutCP437(Ch: Byte);
var
  U: LongInt;
begin
  U := CP437[Ch];
  if U < $80 then
    ConsoleOutChar(Chr(U))
  else if U < $800 then
  begin
    ConsoleOutChar(Chr($C0 or (U shr 6)));
    ConsoleOutChar(Chr($80 or (U and $3F)));
  end
  else
  begin
    ConsoleOutChar(Chr($E0 or (U shr 12)));
    ConsoleOutChar(Chr($80 or ((U shr 6) and $3F)));
    ConsoleOutChar(Chr($80 or (U and $3F)));
  end;
end;

{ ── input ───────────────────────────────────────────────── }

{ Read whatever input arrives within TimeoutMs (-1: wait). }
function ReadInput(TimeoutMs: LongInt): Boolean;
var
  Pfd: TPollFd;
  N: Int64;
begin
  ReadInput := False;
  if InLen >= 1024 then
    Exit;
  Pfd.Fd := 0;
  Pfd.Events := 1;
  Pfd.REvents := 0;
  if c_poll(@Pfd, 1, TimeoutMs) <= 0 then
    Exit;
  N := FpRead(0, @InBuf[InLen], 1024 - InLen);
  if N > 0 then
  begin
    InLen := InLen + N;
    ReadInput := True;
  end;
end;

procedure Consume(N: LongInt);
var
  I: LongInt;
begin
  for I := N to InLen - 1 do
    InBuf[I - N] := InBuf[I];
  InLen := InLen - N;
end;

procedure PushKey(Code: Word; Shift: Byte);
var
  Slot: LongInt;
begin
  if KeyCount >= KeyQueueSize then
    Exit;
  Slot := (KeyHead + KeyCount) mod KeyQueueSize;
  Keys[Slot].Code := Code;
  Keys[Slot].Shift := Shift;
  KeyCount := KeyCount + 1;
end;

procedure ConsolePutMouse(const M: TConsoleMouse);
var
  Slot: LongInt;
begin
  if MouseCount >= MouseQueueSize then
    Exit;
  Slot := (MouseHead + MouseCount) mod MouseQueueSize;
  Mice[Slot] := M;
  MouseCount := MouseCount + 1;
end;

{ The DOS scan code of a character key. }
function ScanOf(C: Char): Byte;
const
  Letters: array[0..25] of Byte = (
    $1E, $30, $2E, $20, $12, $21, $22, $23, $17, $24, $25, $26, $32,
    $31, $18, $19, $10, $13, $1F, $14, $16, $2F, $11, $2D, $15, $2C);
begin
  case C of
    'a'..'z': ScanOf := Letters[Ord(C) - Ord('a')];
    'A'..'Z': ScanOf := Letters[Ord(C) - Ord('A')];
    '1'..'9': ScanOf := Ord(C) - Ord('1') + 2;
    '0': ScanOf := $0B;
    ' ': ScanOf := $39;
    '-', '_': ScanOf := $0C;
    '=', '+': ScanOf := $0D;
    '[', '{': ScanOf := $1A;
    ']', '}': ScanOf := $1B;
    ';', ':': ScanOf := $27;
    '''', '"': ScanOf := $28;
    '`', '~': ScanOf := $29;
    '\', '|': ScanOf := $2B;
    ',', '<': ScanOf := $33;
    '.', '>': ScanOf := $34;
    '/', '?': ScanOf := $35;
  else
    ScanOf := 0;
  end;
end;

procedure PushByte(B: Byte);
begin
  case B of
    13, 10: PushKey($1C0D, 0);
    9: PushKey($0F09, 0);
    8, 127: PushKey($0E08, 0);
    27: PushKey($011B, 0);
    0: PushKey($0300, ConsoleCtrl);
    1..7, 11, 12, 14..26:
      PushKey((ScanOf(Chr(B + 96)) shl 8) or B, ConsoleCtrl);
  else
    if (B >= Ord('A')) and (B <= Ord('Z')) then
      PushKey((ScanOf(Chr(B)) shl 8) or B, ConsoleShift)
    else
      PushKey((ScanOf(Chr(B)) shl 8) or B, 0);
  end;
end;

{ Alt+character, as a terminal sends it: ESC then the character. }
procedure PushAlt(B: Byte);
begin
  if (B >= Ord('0')) and (B <= Ord('9')) then
  begin
    if B = Ord('0') then
      PushKey($8100, ConsoleAlt)
    else
      PushKey(($78 + B - Ord('1')) shl 8, ConsoleAlt);
  end
  else if (B = 8) or (B = 127) then
    PushKey($0E00, ConsoleAlt)
  else if B = 13 then
    PushKey($1C00, ConsoleAlt)
  else
    PushKey(ScanOf(Chr(B)) shl 8, ConsoleAlt);
end;

{ The modifier parameter of a CSI sequence (1 + shift + 2 alt + 4 ctrl)
  as shift bits. }
function ModShift(M: LongInt): Byte;
var
  S: Byte;
begin
  S := 0;
  if M > 1 then
  begin
    M := M - 1;
    if (M and 1) <> 0 then S := S or ConsoleShift;
    if (M and 2) <> 0 then S := S or ConsoleAlt;
    if (M and 4) <> 0 then S := S or ConsoleCtrl;
  end;
  ModShift := S;
end;

{ A navigation key's code with modifiers applied. Base is the plain scan
  code; CtrlScan and AltScan the scan codes with Ctrl and Alt. }
procedure PushNav(Base, CtrlScan, AltScan: Byte; Shift: Byte);
begin
  if ((Shift and ConsoleCtrl) <> 0) and (CtrlScan <> 0) then
    PushKey(CtrlScan shl 8, Shift)
  else if ((Shift and ConsoleAlt) <> 0) and (AltScan <> 0) then
    PushKey(AltScan shl 8, Shift)
  else
    PushKey(Base shl 8, Shift);
end;

{ Function key N (1..12) with modifiers. }
procedure PushFn(N: LongInt; Shift: Byte);
var
  Scan: Byte;
begin
  if N <= 10 then
  begin
    if (Shift and ConsoleAlt) <> 0 then Scan := $68 + N - 1
    else if (Shift and ConsoleCtrl) <> 0 then Scan := $5E + N - 1
    else if (Shift and ConsoleShift) <> 0 then Scan := $54 + N - 1
    else Scan := $3B + N - 1;
  end
  else
  begin
    if (Shift and ConsoleAlt) <> 0 then Scan := $8B + N - 11
    else if (Shift and ConsoleCtrl) <> 0 then Scan := $89 + N - 11
    else if (Shift and ConsoleShift) <> 0 then Scan := $87 + N - 11
    else Scan := $85 + N - 11;
  end;
  PushKey(Scan shl 8, Shift);
end;

procedure MouseReport(B, X, Y: LongInt; Release: Boolean);
var
  M: TConsoleMouse;
  Bit: Word;
begin
  ConsoleMouseX := X;
  ConsoleMouseY := Y;
  M.X := X;
  M.Y := Y;
  if (B and 64) <> 0 then
  begin
    { The wheel: a press of button 4 (up) or 5 (down). }
    if (B and 1) = 0 then Bit := $08 else Bit := $10;
    M.Buttons := ConsoleMouseButtons or Bit;
    M.Action := ConsoleMouseDown;
    ConsolePutMouse(M);
    Exit;
  end;
  case B and 3 of
    0: Bit := 1;
    1: Bit := 4;
    2: Bit := 2;
  else
    Bit := 0;
  end;
  if (B and 32) <> 0 then
    M.Action := ConsoleMouseMove
  else if Release then
  begin
    ConsoleMouseButtons := ConsoleMouseButtons and not Bit;
    M.Action := ConsoleMouseUp;
  end
  else
  begin
    ConsoleMouseButtons := ConsoleMouseButtons or Bit;
    M.Action := ConsoleMouseDown;
  end;
  M.Buttons := ConsoleMouseButtons;
  ConsolePutMouse(M);
end;

{ Decode the escape sequence at the start of the input. Returns its
  length, or 0 if it is not complete yet. }
function DecodeEscape: LongInt;
var
  I, N, P0, P1, P2: LongInt;
  Params: array[0..3] of LongInt;
  Private, Final: Byte;
  Shift: Byte;
begin
  DecodeEscape := 0;
  if InLen < 2 then
    Exit;
  if (InBuf[1] <> Ord('[')) and (InBuf[1] <> Ord('O')) then
  begin
    if InBuf[1] = 27 then
    begin
      PushKey($011B, 0);
      DecodeEscape := 1;
    end
    else
    begin
      PushAlt(InBuf[1]);
      DecodeEscape := 2;
    end;
    Exit;
  end;
  { CSI or SS3: parameters, then a final byte in $40..$7E. }
  for N := 0 to 3 do
    Params[N] := 0;
  N := 0;
  Private := 0;
  I := 2;
  Final := 0;
  while I < InLen do
  begin
    if (InBuf[I] >= Ord('0')) and (InBuf[I] <= Ord('9')) then
    begin
      if N <= 3 then
        Params[N] := Params[N] * 10 + InBuf[I] - Ord('0');
    end
    else if InBuf[I] = Ord(';') then
      N := N + 1
    else if (InBuf[I] = Ord('<')) or (InBuf[I] = Ord('?')) then
      Private := InBuf[I]
    else if (InBuf[I] >= $40) and (InBuf[I] <= $7E) then
    begin
      Final := InBuf[I];
      Break;
    end
    else
    begin
      { Not a sequence we know: treat the ESC as a key. }
      PushKey($011B, 0);
      DecodeEscape := 1;
      Exit;
    end;
    I := I + 1;
  end;
  if Final = 0 then
    Exit;
  DecodeEscape := I + 1;
  P0 := Params[0];
  P1 := Params[1];
  P2 := Params[2];
  if Private = Ord('<') then
  begin
    if (Final = Ord('M')) or (Final = Ord('m')) then
      MouseReport(P0, P1 - 1, P2 - 1, Final = Ord('m'));
    Exit;
  end;
  Shift := ModShift(P1);
  case Chr(Final) of
    'A': PushNav($48, $8D, $98, Shift);
    'B': PushNav($50, $91, $A0, Shift);
    'C': PushNav($4D, $74, $9D, Shift);
    'D': PushNav($4B, $73, $9B, Shift);
    'H': PushNav($47, $77, $97, Shift);
    'F': PushNav($4F, $75, $9F, Shift);
    'Z': PushKey($0F00, ConsoleShift);
    'P': PushFn(1, Shift);
    'Q': PushFn(2, Shift);
    'R': PushFn(3, Shift);
    'S': PushFn(4, Shift);
    '~':
      case P0 of
        1, 7: PushNav($47, $77, $97, Shift);
        2: PushNav($52, $92, $A2, Shift);
        3: PushNav($53, $93, $A3, Shift);
        4, 8: PushNav($4F, $75, $9F, Shift);
        5: PushNav($49, $84, $99, Shift);
        6: PushNav($51, $76, $A1, Shift);
        11..15: PushFn(P0 - 10, Shift);
        17..21: PushFn(P0 - 11, Shift);
        23, 24: PushFn(P0 - 12, Shift);
      end;
  end;
end;

{ Decode a UTF-8 sequence at the start of the input into a code page
  437 character. Returns its length, or 0 if incomplete. }
function DecodeUtf8: LongInt;
var
  Len, I, U: LongInt;
  B: Byte;
begin
  B := InBuf[0];
  if B >= $F0 then Len := 4
  else if B >= $E0 then Len := 3
  else if B >= $C0 then Len := 2
  else Len := 1;
  DecodeUtf8 := 0;
  if InLen < Len then
    Exit;
  DecodeUtf8 := Len;
  if Len = 1 then
  begin
    PushByte(B);
    Exit;
  end;
  if Len = 2 then U := B and $1F
  else if Len = 3 then U := B and $0F
  else U := B and $07;
  for I := 1 to Len - 1 do
    U := (U shl 6) or (InBuf[I] and $3F);
  for I := 128 to 255 do
    if CP437[I] = U then
    begin
      PushKey(I, 0);
      Exit;
    end;
  PushKey(Ord('?'), 0);
end;

procedure ConsoleAddSequence(const Seq: string; Proc: TConsoleProc);
begin
  if SpecialCount > 15 then
    Exit;
  SpecialSeqs[SpecialCount] := Seq;
  SpecialProcs[SpecialCount] := Proc;
  SpecialCount := SpecialCount + 1;
end;

function ConsoleKeyPressed: Boolean;
begin
  if InLen = 0 then
    ReadInput(0);
  ConsoleKeyPressed := InLen > 0;
end;

function ConsoleReadRaw: Char;
begin
  while InLen = 0 do
    ReadInput(-1);
  ConsoleReadRaw := Chr(InBuf[0]);
  Consume(1);
end;

{ The registered sequence the input starts with, or -1. }
function SpecialAt: LongInt;
var
  I, J, Len: LongInt;
  Match: Boolean;
begin
  SpecialAt := -1;
  for I := 0 to SpecialCount - 1 do
  begin
    Len := Length(SpecialSeqs[I]);
    if (Len > 0) and (Len <= InLen) then
    begin
      Match := True;
      for J := 1 to Len do
        if Ord(SpecialSeqs[I][J]) <> InBuf[J - 1] then
          Match := False;
      if Match then
      begin
        SpecialAt := I;
        Exit;
      end;
    end;
  end;
end;

{ Turn the input read so far into keys and mouse events. }
procedure Decode;
var
  N, Special: LongInt;
  Handler: TConsoleProc;
begin
  while InLen > 0 do
  begin
    Special := -1;
    if InBuf[0] = 27 then
      Special := SpecialAt;
    if Special >= 0 then
    begin
      Consume(Length(SpecialSeqs[Special]));
      Handler := SpecialProcs[Special];
      Handler;
      N := 0;
    end
    else if InBuf[0] = 27 then
    begin
      N := DecodeEscape;
      if N = 0 then
      begin
        { Wait briefly for the rest of the sequence; a lone ESC is the
          Esc key. }
        if not ReadInput(30) then
        begin
          PushKey($011B, 0);
          N := 1;
        end;
      end;
    end
    else if InBuf[0] >= $80 then
    begin
      N := DecodeUtf8;
      if (N = 0) and not ReadInput(30) then
        N := InLen;
    end
    else
    begin
      PushByte(InBuf[0]);
      N := 1;
    end;
    if N > 0 then
      Consume(N);
  end;
end;

procedure Pump(TimeoutMs: LongInt);
begin
  if ReadInput(TimeoutMs) then
    Decode;
end;

function ConsolePeekKey(var K: TConsoleKey): Boolean;
begin
  if KeyCount = 0 then
    Pump(0);
  ConsolePeekKey := KeyCount > 0;
  if KeyCount > 0 then
    K := Keys[KeyHead];
end;

procedure ConsoleGetKey(var K: TConsoleKey);
begin
  while KeyCount = 0 do
    Pump(-1);
  K := Keys[KeyHead];
  KeyHead := (KeyHead + 1) mod KeyQueueSize;
  KeyCount := KeyCount - 1;
end;

function ConsolePeekMouse(var M: TConsoleMouse): Boolean;
begin
  if MouseCount = 0 then
    Pump(0);
  ConsolePeekMouse := MouseCount > 0;
  if MouseCount > 0 then
    M := Mice[MouseHead];
end;

procedure ConsoleGetMouse(var M: TConsoleMouse);
begin
  while MouseCount = 0 do
    Pump(-1);
  M := Mice[MouseHead];
  MouseHead := (MouseHead + 1) mod MouseQueueSize;
  MouseCount := MouseCount - 1;
end;

end.
