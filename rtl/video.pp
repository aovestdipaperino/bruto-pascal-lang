{ Bruto Pascal runtime: FPC's Video unit on an ANSI terminal. The screen
  is a buffer of cells (character in the low byte, attribute in the high
  one, code page 437); UpdateScreen sends the cells that changed.

  After InitEnhancedVideo (Free Vision's Unicode build) the screen is
  EnhancedVideoBuf instead: each cell holds an extended grapheme cluster
  and an attribute, and a wide cluster covers the cell to its right too. }
unit Video;

interface

uses BrutoConsole, GraphemeBreakProperty;

type
  PVideoMode = ^TVideoMode;
  TVideoMode = record
    Col, Row: Word;
    Color: Boolean;
  end;
  TVideoCell = Word;
  PVideoCell = ^TVideoCell;
  TVideoBuf = array[0..32759] of TVideoCell;
  PVideoBuf = ^TVideoBuf;
  TSystemCodePage = Word;

  { FPC's cell also has 24-bit colors and attributes such as italics,
    behind properties; Free Vision uses only these two. }
  TEnhancedVideoCell = record
    ExtendedGraphemeCluster: UnicodeString;
    Attribute: Byte;
  end;
  PEnhancedVideoCell = ^TEnhancedVideoCell;
  TEnhancedVideoCells = array[0..32759] of TEnhancedVideoCell;
  { A dynamic array in FPC; Free Vision only takes its address. }
  TEnhancedVideoBuf = ^TEnhancedVideoCells;

const
  Black = 0;
  Blue = 1;
  Green = 2;
  Cyan = 3;
  Red = 4;
  Magenta = 5;
  Brown = 6;
  LightGray = 7;
  DarkGray = 8;
  LightBlue = 9;
  LightGreen = 10;
  LightCyan = 11;
  LightRed = 12;
  LightMagenta = 13;
  Yellow = 14;
  White = 15;
  Blink = 128;

  cpUnderLine = $0001;
  cpBlink = $0002;
  cpColor = $0004;
  cpChangeFont = $0008;
  cpChangeMode = $0010;
  cpChangeCursor = $0020;

  crHidden = 0;
  crUnderLine = 1;
  crBlock = 2;
  crHalfBlock = 3;

  vioOK = 0;
  errVioBase = 1000;
  errVioInit = errVioBase + 1;
  errVioNotSupported = errVioBase + 2;
  errVioNoSuchMode = errVioBase + 3;
  errOk = 0;

  LowAscii = True;
  NoExtendedFrame = False;
  FVMaxWidth = 1024;

var
  ScreenWidth, ScreenHeight: Word;
  ScreenColor: Boolean;
  CursorX, CursorY: Word;
  VideoBuf, OldVideoBuf: PVideoBuf;
  VideoBufSize: LongInt;
  CursorLines: Byte;
  ErrorCode: LongInt;
  ErrorInfo: Pointer;
  EnhancedVideoBuf, OldEnhancedVideoBuf: TEnhancedVideoBuf;

procedure InitVideo;
procedure DoneVideo;
procedure InitEnhancedVideo;
procedure DoneEnhancedVideo;
function GetCapabilities: Word;
procedure ClearScreen;
procedure UpdateScreen(Force: Boolean);
procedure UpdateScreenArea(const X1, Y1, X2, Y2: Word; Force: Boolean);
procedure SetCursorPos(NewCursorX, NewCursorY: Word);
function GetCursorType: Word;
procedure SetCursorType(NewType: Word);
procedure LockScreenUpdate;
procedure UnlockScreenUpdate;
function GetLockScreenCount: Integer;
procedure GetVideoMode(var Mode: TVideoMode);
function SetVideoMode(const Mode: TVideoMode): Boolean;
function GetVideoModeCount: Word;
function GetVideoModeData(Index: Word; var Data: TVideoMode): Boolean;
{ Cells always hold code page 437 characters. }
function GetActiveCodePage: TSystemCodePage;
function GetLegacyCodePage: TSystemCodePage;
procedure ActivateCodePage(CodePage: TSystemCodePage);
function GetSupportedCodePageCount: Integer;
function GetSupportedCodePage(Index: Integer): TSystemCodePage;
{ How many cells a cluster takes on the screen: 2 for wide characters
  and emoji, else 1. }
function ExtendedGraphemeClusterDisplayWidth(const EGC: UnicodeString): Integer;
{ How many cells S takes on the screen. }
function StringDisplayWidth(const S: UnicodeString): Integer;

implementation

const
  { DOS color number -> ANSI color number. }
  AnsiColor: array[0..7] of Byte = (0, 4, 2, 6, 1, 5, 3, 7);

var
  Initialized: Boolean;
  Enhanced: Boolean;
  EnhancedCells: LongInt;
  LockCount: Integer;
  CursorType: Word;
  LastAttr: LongInt;

procedure OutAttr(Attr: Byte);
var
  Fg, Bg: Byte;
begin
  Fg := Attr and $0F;
  Bg := (Attr shr 4) and $07;
  ConsoleOutStr(#27'[0;');
  if Fg >= 8 then
    ConsoleOutNum(90 + AnsiColor[Fg - 8])
  else
    ConsoleOutNum(30 + AnsiColor[Fg]);
  ConsoleOutChar(';');
  if (Attr and $80) <> 0 then
    ConsoleOutNum(100 + AnsiColor[Bg])
  else
    ConsoleOutNum(40 + AnsiColor[Bg]);
  ConsoleOutChar('m');
end;

procedure OutGoto(X, Y: LongInt);
begin
  ConsoleOutStr(#27'[');
  ConsoleOutNum(Y + 1);
  ConsoleOutChar(';');
  ConsoleOutNum(X + 1);
  ConsoleOutChar('H');
end;

procedure OutCursor;
begin
  if (CursorType = crHidden) or (CursorX >= ScreenWidth) or (CursorY >= ScreenHeight) then
    ConsoleOutStr(#27'[?25l')
  else
  begin
    OutGoto(CursorX, CursorY);
    ConsoleOutStr(#27'[?25h');
  end;
end;

procedure AllocBuffers;
var
  I: LongInt;
begin
  VideoBufSize := LongInt(ScreenWidth) * ScreenHeight * SizeOf(TVideoCell);
  GetMem(VideoBuf, VideoBufSize);
  GetMem(OldVideoBuf, VideoBufSize);
  for I := 0 to LongInt(ScreenWidth) * ScreenHeight - 1 do
  begin
    VideoBuf^[I] := $0720;
    OldVideoBuf^[I] := $0720;
  end;
end;

procedure InitVideo;
begin
  ErrorCode := vioOK;
  if Initialized then
    Exit;
  ConsoleSize(ScreenWidth, ScreenHeight);
  ScreenColor := True;
  CursorX := 0;
  CursorY := 0;
  CursorType := crUnderLine;
  CursorLines := 0;
  AllocBuffers;
  { The alternate screen, cleared, with the cursor hidden. }
  ConsoleOutStr(#27'[?1049h'#27'[0m'#27'[2J'#27'[?25l');
  ConsoleFlush;
  LastAttr := -1;
  Initialized := True;
end;

procedure DoneVideo;
begin
  if not Initialized then
    Exit;
  ConsoleOutStr(#27'[0m'#27'[?25h'#27'[?1049l');
  ConsoleFlush;
  FreeMem(VideoBuf, VideoBufSize);
  FreeMem(OldVideoBuf, VideoBufSize);
  VideoBuf := nil;
  OldVideoBuf := nil;
  Initialized := False;
end;

function ExtendedGraphemeClusterDisplayWidth(const EGC: UnicodeString): Integer;
var
  Units: Int64;
begin
  if Length(EGC) = 0 then
    ExtendedGraphemeClusterDisplayWidth := 1
  else if IsWideCodePoint(CodePointAt(EGC, 1, Units)) or (Pos(WideChar($FE0F), EGC) > 0) then
    ExtendedGraphemeClusterDisplayWidth := 2
  else
    ExtendedGraphemeClusterDisplayWidth := 1;
end;

function StringDisplayWidth(const S: UnicodeString): Integer;
var
  EGC: UnicodeString;
  W: Integer;
begin
  W := 0;
  for EGC in TUnicodeStringExtendedGraphemeClustersEnumerator.Create(S) do
    W := W + ExtendedGraphemeClusterDisplayWidth(EGC);
  StringDisplayWidth := W;
end;

procedure FillEnhanced(Buf: TEnhancedVideoBuf);
var
  I: LongInt;
begin
  for I := 0 to EnhancedCells - 1 do
  begin
    Buf^[I].ExtendedGraphemeCluster := ' ';
    Buf^[I].Attribute := $07;
  end;
end;

procedure InitEnhancedVideo;
begin
  InitVideo;
  if Enhanced then
    Exit;
  EnhancedCells := LongInt(ScreenWidth) * ScreenHeight;
  GetMem(EnhancedVideoBuf, EnhancedCells * SizeOf(TEnhancedVideoCell));
  GetMem(OldEnhancedVideoBuf, EnhancedCells * SizeOf(TEnhancedVideoCell));
  FillChar(EnhancedVideoBuf^, EnhancedCells * SizeOf(TEnhancedVideoCell), 0);
  FillChar(OldEnhancedVideoBuf^, EnhancedCells * SizeOf(TEnhancedVideoCell), 0);
  FillEnhanced(EnhancedVideoBuf);
  FillEnhanced(OldEnhancedVideoBuf);
  Enhanced := True;
end;

procedure DoneEnhancedVideo;
begin
  if Enhanced then
  begin
    FreeMem(EnhancedVideoBuf, EnhancedCells * SizeOf(TEnhancedVideoCell));
    FreeMem(OldEnhancedVideoBuf, EnhancedCells * SizeOf(TEnhancedVideoCell));
    EnhancedVideoBuf := nil;
    OldEnhancedVideoBuf := nil;
    Enhanced := False;
  end;
  DoneVideo;
end;

function GetCapabilities: Word;
begin
  GetCapabilities := cpColor or cpChangeCursor;
end;

procedure ClearScreen;
var
  I: LongInt;
begin
  if not Initialized then
    Exit;
  for I := 0 to LongInt(ScreenWidth) * ScreenHeight - 1 do
    VideoBuf^[I] := $0720;
  if Enhanced then
    FillEnhanced(EnhancedVideoBuf);
  UpdateScreen(True);
end;

procedure SendCells(X1, Y1, X2, Y2: LongInt; Force: Boolean);
var
  X, Y, I, NextX, NextY: LongInt;
  Cell: Word;
begin
  NextX := -1;
  NextY := -1;
  for Y := Y1 to Y2 do
    for X := X1 to X2 do
    begin
      I := Y * ScreenWidth + X;
      Cell := VideoBuf^[I];
      if Force or (Cell <> OldVideoBuf^[I]) then
      begin
        if (X <> NextX) or (Y <> NextY) then
          OutGoto(X, Y);
        if (Cell shr 8) <> LastAttr then
        begin
          OutAttr(Cell shr 8);
          LastAttr := Cell shr 8;
        end;
        ConsoleOutCP437(Cell and $FF);
        OldVideoBuf^[I] := Cell;
        NextX := X + 1;
        NextY := Y;
      end;
    end;
end;

{ Write one cluster: control characters as their code page 437
  pictures, as the legacy cells show them. }
procedure OutCluster(const EGC: UnicodeString);
var
  S: AnsiString;
begin
  if Length(EGC) = 0 then
    ConsoleOutChar(' ')
  else if (Length(EGC) = 1) and (Ord(EGC[1]) < 32) then
    ConsoleOutCP437(Ord(EGC[1]))
  else
  begin
    S := EGC;
    ConsoleOutStr(S);
  end;
end;

procedure SendEnhancedCells(X1, Y1, X2, Y2: LongInt; Force: Boolean);
var
  X, Y, I, W, OldW, NextX, NextY: LongInt;
  Redraw: Boolean;
begin
  NextX := -1;
  NextY := -1;
  for Y := Y1 to Y2 do
  begin
    { A wide cluster that changed (or went) leaves its right half to
      redraw. }
    Redraw := False;
    X := X1;
    { Start at the cluster covering X1. }
    if (X > 0) and (ExtendedGraphemeClusterDisplayWidth(
      EnhancedVideoBuf^[Y * ScreenWidth + X - 1].ExtendedGraphemeCluster) = 2) then
      X := X - 1;
    while X <= X2 do
    begin
      I := Y * ScreenWidth + X;
      W := ExtendedGraphemeClusterDisplayWidth(EnhancedVideoBuf^[I].ExtendedGraphemeCluster);
      if X + W > ScreenWidth then
        W := 1;
      if Force or Redraw
        or (EnhancedVideoBuf^[I].Attribute <> OldEnhancedVideoBuf^[I].Attribute)
        or (EnhancedVideoBuf^[I].ExtendedGraphemeCluster <> OldEnhancedVideoBuf^[I].ExtendedGraphemeCluster) then
      begin
        OldW := ExtendedGraphemeClusterDisplayWidth(OldEnhancedVideoBuf^[I].ExtendedGraphemeCluster);
        if (X <> NextX) or (Y <> NextY) then
          OutGoto(X, Y);
        if EnhancedVideoBuf^[I].Attribute <> LastAttr then
        begin
          OutAttr(EnhancedVideoBuf^[I].Attribute);
          LastAttr := EnhancedVideoBuf^[I].Attribute;
        end;
        OutCluster(EnhancedVideoBuf^[I].ExtendedGraphemeCluster);
        OldEnhancedVideoBuf^[I] := EnhancedVideoBuf^[I];
        NextX := X + W;
        NextY := Y;
        Redraw := (OldW = 2) and (W = 1);
      end
      else
        Redraw := False;
      { The cell a wide cluster covers is not drawn by itself. }
      if W = 2 then
      begin
        OldEnhancedVideoBuf^[I + 1] := EnhancedVideoBuf^[I + 1];
        Redraw := False;
      end;
      X := X + W;
    end;
  end;
end;

procedure UpdateScreen(Force: Boolean);
begin
  if (not Initialized) or (LockCount > 0) then
    Exit;
  if Force then
    LastAttr := -1;
  if Enhanced then
    SendEnhancedCells(0, 0, ScreenWidth - 1, ScreenHeight - 1, Force)
  else
    SendCells(0, 0, ScreenWidth - 1, ScreenHeight - 1, Force);
  OutCursor;
  ConsoleFlush;
end;

procedure UpdateScreenArea(const X1, Y1, X2, Y2: Word; Force: Boolean);
begin
  if (not Initialized) or (LockCount > 0) then
    Exit;
  if (X2 >= ScreenWidth) or (Y2 >= ScreenHeight) or (X1 > X2) or (Y1 > Y2) then
    Exit;
  if Enhanced then
    SendEnhancedCells(X1, Y1, X2, Y2, Force)
  else
    SendCells(X1, Y1, X2, Y2, Force);
  OutCursor;
  ConsoleFlush;
end;

procedure SetCursorPos(NewCursorX, NewCursorY: Word);
begin
  CursorX := NewCursorX;
  CursorY := NewCursorY;
  if Initialized and (LockCount = 0) then
  begin
    OutCursor;
    ConsoleFlush;
  end;
end;

function GetCursorType: Word;
begin
  GetCursorType := CursorType;
end;

procedure SetCursorType(NewType: Word);
begin
  CursorType := NewType;
  if Initialized and (LockCount = 0) then
  begin
    OutCursor;
    ConsoleFlush;
  end;
end;

procedure LockScreenUpdate;
begin
  LockCount := LockCount + 1;
end;

procedure UnlockScreenUpdate;
begin
  if LockCount > 0 then
    LockCount := LockCount - 1;
end;

function GetLockScreenCount: Integer;
begin
  GetLockScreenCount := LockCount;
end;

procedure GetVideoMode(var Mode: TVideoMode);
begin
  Mode.Col := ScreenWidth;
  Mode.Row := ScreenHeight;
  Mode.Color := ScreenColor;
end;

function SetVideoMode(const Mode: TVideoMode): Boolean;
begin
  { The terminal decides the size; only the current mode is valid. }
  SetVideoMode := (Mode.Col = ScreenWidth) and (Mode.Row = ScreenHeight);
end;

function GetVideoModeCount: Word;
begin
  GetVideoModeCount := 1;
end;

function GetVideoModeData(Index: Word; var Data: TVideoMode): Boolean;
begin
  GetVideoModeData := Index = 0;
  if Index = 0 then
    GetVideoMode(Data);
end;

function GetActiveCodePage: TSystemCodePage;
begin
  GetActiveCodePage := 437;
end;

function GetLegacyCodePage: TSystemCodePage;
begin
  GetLegacyCodePage := 437;
end;

procedure ActivateCodePage(CodePage: TSystemCodePage);
begin
end;

function GetSupportedCodePageCount: Integer;
begin
  GetSupportedCodePageCount := 1;
end;

function GetSupportedCodePage(Index: Integer): TSystemCodePage;
begin
  GetSupportedCodePage := 437;
end;

end.
