{ Bruto Pascal runtime: FPC's Mouse unit on a terminal that reports the
  mouse with SGR (1006) sequences. }
unit Mouse;

interface

uses BrutoConsole;

const
  errMouseBase = 1030;
  errMouseInitError = errMouseBase + 0;
  errMouseNotImplemented = errMouseBase + 1;

type
  PMouseEvent = ^TMouseEvent;
  TMouseEvent = packed record
    Buttons: Word;
    X, Y: Word;
    Action: Word;
  end;

const
  MouseActionDown = $0001;
  MouseActionUp = $0002;
  MouseActionMove = $0004;

  MouseLeftButton = $01;
  MouseRightButton = $02;
  MouseMiddleButton = $04;
  MouseButton4 = $08;
  MouseButton5 = $10;
  MouseXButton1 = $20;
  MouseXButton2 = $40;

  MouseEventBufSize = 16;

var
  MouseIntFlag: Byte;
  MouseButtons: Word;
  MouseWhereX, MouseWhereY: Word;

procedure InitMouse;
procedure DoneMouse;
function DetectMouse: Byte;
procedure ShowMouse;
procedure HideMouse;
function GetMouseX: Word;
function GetMouseY: Word;
function GetMouseButtons: Word;
procedure SetMouseXY(X, Y: Word);
procedure GetMouseEvent(var MouseEvent: TMouseEvent);
procedure PutMouseEvent(const MouseEvent: TMouseEvent);
function PollMouseEvent(var MouseEvent: TMouseEvent): Boolean;

implementation

var
  Enabled: Boolean;

procedure InitMouse;
begin
  if Enabled then
    Exit;
  { Report presses, drags and motion, in SGR form. }
  ConsoleOutStr(#27'[?1000h'#27'[?1002h'#27'[?1003h'#27'[?1006h');
  ConsoleFlush;
  Enabled := True;
end;

procedure DoneMouse;
begin
  if not Enabled then
    Exit;
  ConsoleOutStr(#27'[?1006l'#27'[?1003l'#27'[?1002l'#27'[?1000l');
  ConsoleFlush;
  Enabled := False;
end;

function DetectMouse: Byte;
begin
  DetectMouse := 3;
end;

procedure ShowMouse;
begin
end;

procedure HideMouse;
begin
end;

function GetMouseX: Word;
begin
  GetMouseX := ConsoleMouseX;
end;

function GetMouseY: Word;
begin
  GetMouseY := ConsoleMouseY;
end;

function GetMouseButtons: Word;
begin
  GetMouseButtons := ConsoleMouseButtons;
end;

procedure SetMouseXY(X, Y: Word);
begin
  ConsoleMouseX := X;
  ConsoleMouseY := Y;
end;

procedure FromConsole(const M: TConsoleMouse; var MouseEvent: TMouseEvent);
begin
  MouseEvent.Buttons := M.Buttons;
  MouseEvent.X := M.X;
  MouseEvent.Y := M.Y;
  MouseEvent.Action := M.Action;
  MouseButtons := M.Buttons;
  MouseWhereX := M.X;
  MouseWhereY := M.Y;
end;

procedure GetMouseEvent(var MouseEvent: TMouseEvent);
var
  M: TConsoleMouse;
begin
  ConsoleGetMouse(M);
  FromConsole(M, MouseEvent);
end;

procedure PutMouseEvent(const MouseEvent: TMouseEvent);
var
  M: TConsoleMouse;
begin
  M.Buttons := MouseEvent.Buttons;
  M.X := MouseEvent.X;
  M.Y := MouseEvent.Y;
  M.Action := MouseEvent.Action;
  ConsolePutMouse(M);
end;

function PollMouseEvent(var MouseEvent: TMouseEvent): Boolean;
var
  M: TConsoleMouse;
  Found: Boolean;
begin
  Found := ConsolePeekMouse(M);
  if Found then
    FromConsole(M, MouseEvent);
  PollMouseEvent := Found;
end;

end.
