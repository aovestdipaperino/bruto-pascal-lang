{ Bruto Pascal runtime: FPC's Keyboard unit on a raw-mode terminal. Keys
  arrive with DOS scan codes (see BrutoConsole), as Free Vision expects. }
unit Keyboard;

interface

uses BrutoConsole;

const
  errKbdBase = 1010;
  errKbdInitError = errKbdBase + 0;
  errKbdNotImplemented = errKbdBase + 1;

type
  TKeyEvent = LongWord;

  TKeyRecord = packed record
    KeyCode: Word;
    ShiftState, Flags: Byte;
  end;

  TEnhancedShiftStateElement = (
    essShift, essLeftShift, essRightShift,
    essCtrl, essLeftCtrl, essRightCtrl,
    essAlt, essLeftAlt, essRightAlt, essAltGr,
    essCapsLockPressed, essCapsLockOn,
    essNumLockPressed, essNumLockOn,
    essScrollLockPressed, essScrollLockOn);
  TEnhancedShiftState = set of TEnhancedShiftStateElement;

  TEnhancedKeyEvent = record
    VirtualKeyCode: Word;
    VirtualScanCode: Word;
    UnicodeChar: WideChar;
    AsciiChar: AnsiChar;
    ShiftState: TEnhancedShiftState;
    Flags: Byte;
  end;

const
  NilEnhancedKeyEvent: TEnhancedKeyEvent = (
    VirtualKeyCode: 0;
    VirtualScanCode: 0;
    UnicodeChar: #0;
    AsciiChar: #0;
    ShiftState: [];
    Flags: 0);

  kbdF1 = $FF01;
  kbdF2 = $FF02;
  kbdF3 = $FF03;
  kbdF4 = $FF04;
  kbdF5 = $FF05;
  kbdF6 = $FF06;
  kbdF7 = $FF07;
  kbdF8 = $FF08;
  kbdF9 = $FF09;
  kbdF10 = $FF0A;
  kbdF11 = $FF0B;
  kbdF12 = $FF0C;
  kbdHome = $FF20;
  kbdUp = $FF21;
  kbdPgUp = $FF22;
  kbdLeft = $FF23;
  kbdMiddle = $FF24;
  kbdRight = $FF25;
  kbdEnd = $FF26;
  kbdDown = $FF27;
  kbdPgDn = $FF28;
  kbdInsert = $FF29;
  kbdDelete = $FF2A;

  kbASCII = $00;
  kbUniCode = $01;
  kbFnKey = $02;
  kbPhys = $03;
  kbReleased = $04;

  kbLeftShift = 1;
  kbRightShift = 2;
  kbShift = kbLeftShift or kbRightShift;
  kbCtrl = 4;
  kbAlt = 8;

procedure InitKeyboard;
procedure DoneKeyboard;
function GetKeyEvent: TKeyEvent;
procedure PutKeyEvent(KeyEvent: TKeyEvent);
function PollKeyEvent: TKeyEvent;
function PollShiftStateEvent: TKeyEvent;
function TranslateKeyEvent(KeyEvent: TKeyEvent): TKeyEvent;
function GetKeyEventFlags(KeyEvent: TKeyEvent): Byte;
function GetKeyEventChar(KeyEvent: TKeyEvent): AnsiChar;
function GetKeyEventUniCode(KeyEvent: TKeyEvent): Word;
function GetKeyEventCode(KeyEvent: TKeyEvent): Word;
function GetKeyEventShiftState(KeyEvent: TKeyEvent): Byte;
function IsFunctionKey(KeyEvent: TKeyEvent): Boolean;
function GetEnhancedKeyEvent: TEnhancedKeyEvent;
function PollEnhancedKeyEvent: TEnhancedKeyEvent;
function ConvertToLegacyKeyEvent(const KeyEvent: TEnhancedKeyEvent): TKeyEvent;
function ConvertEnhancedToLegacyShiftState(const ShiftState: TEnhancedShiftState): Byte;

{ The Unix keyboard unit's raw access, which Free Vision's clipboard
  support uses to read terminal replies. }
type
  TProcedure = procedure;
function KeyPressed: Boolean;
function RawReadKey: AnsiChar;
function AddSpecialSequence(const St: string; Proc: TProcedure): Pointer;

implementation

var
  Pending: TKeyEvent;
  HasPending: Boolean;
  { The modifiers of the last key read: a terminal reports them only
    along with a key, so this is the best answer to GetShiftState. }
  LastShift: Byte;

procedure InitKeyboard;
begin
  ConsoleRawMode;
end;

procedure DoneKeyboard;
begin
  ConsoleRestoreMode;
end;

{ Console shift bits -> the legacy byte (kbShift, kbCtrl, kbAlt). }
function LegacyShift(Shift: Byte): Byte;
var
  S: Byte;
begin
  S := 0;
  if (Shift and ConsoleShift) <> 0 then S := S or kbLeftShift;
  if (Shift and ConsoleCtrl) <> 0 then S := S or kbCtrl;
  if (Shift and ConsoleAlt) <> 0 then S := S or kbAlt;
  LegacyShift := S;
end;

function ToKeyEvent(const K: TConsoleKey): TKeyEvent;
begin
  ToKeyEvent := K.Code or (LongWord(LegacyShift(K.Shift)) shl 16)
    or (LongWord(kbPhys) shl 24);
end;

function ToEnhanced(const K: TConsoleKey): TEnhancedKeyEvent;
var
  E: TEnhancedKeyEvent;
begin
  E.VirtualKeyCode := K.Code;
  E.VirtualScanCode := K.Code;
  E.AsciiChar := Chr(K.Code and $FF);
  E.UnicodeChar := WideChar(K.Unicode);
  E.ShiftState := [];
  if (K.Shift and ConsoleShift) <> 0 then
    E.ShiftState := E.ShiftState + [essShift, essLeftShift];
  if (K.Shift and ConsoleCtrl) <> 0 then
    E.ShiftState := E.ShiftState + [essCtrl, essLeftCtrl];
  if (K.Shift and ConsoleAlt) <> 0 then
    E.ShiftState := E.ShiftState + [essAlt, essLeftAlt];
  E.Flags := 0;
  ToEnhanced := E;
end;

function GetKeyEvent: TKeyEvent;
var
  K: TConsoleKey;
begin
  if HasPending then
  begin
    HasPending := False;
    LastShift := (Pending shr 16) and $FF;
    GetKeyEvent := Pending;
    Exit;
  end;
  ConsoleGetKey(K);
  LastShift := LegacyShift(K.Shift);
  GetKeyEvent := ToKeyEvent(K);
end;

procedure PutKeyEvent(KeyEvent: TKeyEvent);
begin
  Pending := KeyEvent;
  HasPending := True;
end;

function PollKeyEvent: TKeyEvent;
var
  K: TConsoleKey;
begin
  if HasPending then
    PollKeyEvent := Pending
  else if ConsolePeekKey(K) then
    PollKeyEvent := ToKeyEvent(K)
  else
    PollKeyEvent := 0;
end;

function PollShiftStateEvent: TKeyEvent;
begin
  PollShiftStateEvent := LongWord(LastShift) shl 16;
end;

function TranslateKeyEvent(KeyEvent: TKeyEvent): TKeyEvent;
begin
  TranslateKeyEvent := KeyEvent;
end;

function GetKeyEventFlags(KeyEvent: TKeyEvent): Byte;
begin
  GetKeyEventFlags := (KeyEvent shr 24) and $FF;
end;

function GetKeyEventChar(KeyEvent: TKeyEvent): AnsiChar;
begin
  GetKeyEventChar := Chr(KeyEvent and $FF);
end;

function GetKeyEventUniCode(KeyEvent: TKeyEvent): Word;
begin
  GetKeyEventUniCode := KeyEvent and $FF;
end;

function GetKeyEventCode(KeyEvent: TKeyEvent): Word;
begin
  GetKeyEventCode := KeyEvent and $FFFF;
end;

function GetKeyEventShiftState(KeyEvent: TKeyEvent): Byte;
begin
  GetKeyEventShiftState := (KeyEvent shr 16) and $FF;
end;

function IsFunctionKey(KeyEvent: TKeyEvent): Boolean;
begin
  IsFunctionKey := (KeyEvent and $FF) = 0;
end;

function GetEnhancedKeyEvent: TEnhancedKeyEvent;
var
  K: TConsoleKey;
begin
  ConsoleGetKey(K);
  LastShift := LegacyShift(K.Shift);
  GetEnhancedKeyEvent := ToEnhanced(K);
end;

function PollEnhancedKeyEvent: TEnhancedKeyEvent;
var
  K: TConsoleKey;
begin
  if ConsolePeekKey(K) then
    PollEnhancedKeyEvent := ToEnhanced(K)
  else
    PollEnhancedKeyEvent := NilEnhancedKeyEvent;
end;

function ConvertEnhancedToLegacyShiftState(const ShiftState: TEnhancedShiftState): Byte;
var
  S: Byte;
begin
  S := 0;
  if essAlt in ShiftState then S := S or kbAlt;
  if essCtrl in ShiftState then S := S or kbCtrl;
  if essShift in ShiftState then
  begin
    if ([essLeftShift, essRightShift] * ShiftState) = [] then
      S := S or kbShift
    else
    begin
      if essLeftShift in ShiftState then S := S or kbLeftShift;
      if essRightShift in ShiftState then S := S or kbRightShift;
    end;
  end;
  if essAltGr in ShiftState then
    S := S or kbAlt or kbCtrl;
  ConvertEnhancedToLegacyShiftState := S;
end;

function ConvertToLegacyKeyEvent(const KeyEvent: TEnhancedKeyEvent): TKeyEvent;
begin
  ConvertToLegacyKeyEvent := KeyEvent.VirtualScanCode
    or (LongWord(ConvertEnhancedToLegacyShiftState(KeyEvent.ShiftState)) shl 16)
    or (LongWord(kbPhys) shl 24);
end;

function KeyPressed: Boolean;
begin
  KeyPressed := ConsoleKeyPressed;
end;

function RawReadKey: AnsiChar;
begin
  RawReadKey := ConsoleReadRaw;
end;

function AddSpecialSequence(const St: string; Proc: TProcedure): Pointer;
begin
  ConsoleAddSequence(St, Proc);
  AddSpecialSequence := nil;
end;

end.
