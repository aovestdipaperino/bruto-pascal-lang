{ Bruto Pascal runtime: the part of FPC's TermIO unit Free Vision uses. }
unit TermIO;

interface

uses UnixType;

type
  TWinSize = record
    ws_row, ws_col, ws_xpixel, ws_ypixel: Word;
  end;
  PWinSize = ^TWinSize;

const
{$ifdef DARWIN}
  TIOCGWINSZ = $40087468;
{$else}
  TIOCGWINSZ = $5413;
{$endif}

function IsATTY(Handle: cint): cint; cdecl; external 'c' name 'isatty';
function TTYName(Handle: cint): string;

implementation

function c_ttyname(Handle: cint): PChar; cdecl; external 'c' name 'ttyname';

function TTYName(Handle: cint): string;
var
  P: PChar;
begin
  P := c_ttyname(Handle);
  if P = nil then
    TTYName := ''
  else
    TTYName := P;
end;

end.
