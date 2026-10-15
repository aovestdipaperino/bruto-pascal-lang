{ Bruto Pascal runtime: TObject, the root of every class, and TClass,
  as FPC's objpas declares them. Added to every program that declares a
  class. In a program that has a TObject of its own (Free Vision's
  Objects unit declares an object type of that name), this one is
  BrutoTObject, and classes declared without a parent still descend
  from it.

  A class's VMT starts with the instance size, its negation, a link to
  the parent's VMT, the class name and the routine that releases the
  strings an instance holds; the virtual methods follow. }
unit ObjPas;

interface

type
  TClass = class of TObject;

  { A method pointer's parts: `TMethod(OnClick).Data` is the instance. }
  TMethod = record
    Code, Data: Pointer;
  end;

  TObject = class
  public
    constructor Create;
    destructor Destroy; virtual;
    { Destroy, unless Self is nil. }
    procedure Free;
    class function ClassName: ShortString;
    class function ClassNameIs(const Name: AnsiString): Boolean;
    class function ClassParent: TClass;
    class function InstanceSize: SizeInt;
    class function InheritsFrom(AClass: TClass): Boolean;
    function ClassType: TClass;
    procedure AfterConstruction; virtual;
    procedure BeforeDestruction; virtual;
    function Equals(Obj: TObject): Boolean; virtual;
    function GetHashCode: PtrInt; virtual;
    function ToString: AnsiString; virtual;
  end;

  { Called with a runtime error's code before the program stops; SysUtils
    raises the matching exception from it. }
  TBrutoRunErrorHook = procedure(Code: LongInt);

var
  { The innermost exception frame (one per `try` being run). }
  BrutoExcTop: Pointer;
  { The exception being raised or handled. }
  BrutoExcObject: TObject;
  { The newest record of a running routine's string locals, which an
    exception leaving it releases. }
  BrutoCleanupTop: Pointer;
  BrutoRunErrorHook: TBrutoRunErrorHook;

{ Raise Obj: go back to the innermost `try`, or report it and stop. }
procedure BrutoRaise(Obj: TObject; Address: Pointer);
{ Free a handled exception. }
procedure BrutoFreeException(Obj: TObject);

implementation

type
  PBrutoExcFrame = ^TBrutoExcFrame;
  TBrutoExcFrame = record
    Buf: array[0..63] of Int64;
    Prev: PBrutoExcFrame;
    Cleanup: Pointer;
  end;
  { A routine's string locals: Count slot addresses follow. }
  PBrutoCleanup = ^TBrutoCleanup;
  TBrutoCleanup = record
    Prev: PBrutoCleanup;
    Count: PtrInt;
  end;

{$ifdef WINDOWS}
procedure bruto_longjmp(Buf: Pointer; Value: LongInt); cdecl; external 'c' name 'longjmp';
{$else}
procedure bruto_longjmp(Buf: Pointer; Value: LongInt); cdecl; external 'c' name '_longjmp';
{$endif}
{$ifdef WINDOWS}
function bruto_exc_write(Fd: LongInt; Buf: PChar; Count: LongWord): LongInt; cdecl;
  external 'c' name '_write';
{$else}
function bruto_exc_write(Fd: LongInt; Buf: PChar; Count: PtrUInt): PtrInt; cdecl;
  external 'c' name 'write';
{$endif}
function bruto_exc_fflush(Stream: Pointer): LongInt; cdecl; external 'c' name 'fflush';

procedure BrutoUnhandled(Obj: TObject; Address: Pointer);
var
  Where, Report: AnsiString;
begin
  Where := '$' + HexStr(PtrUInt(Address), 16);
  Report := 'An unhandled exception occurred at ' + Where + ':'#10 + Obj.ToString + #10 +
    '  ' + Where + #10#10;
  bruto_exc_fflush(nil);
  bruto_exc_write(2, PChar(Report), Length(Report));
  Halt(217);
end;

procedure BrutoRaise(Obj: TObject; Address: Pointer);
var
  Frame: PBrutoExcFrame;
  Rec: PBrutoCleanup;
  Slot: Pointer;
  I: PtrInt;
begin
  Frame := BrutoExcTop;
  BrutoExcObject := Obj;
  if Frame = nil then
    BrutoUnhandled(Obj, Address);
  BrutoExcTop := Frame^.Prev;
  { Release the strings of the routines being left. }
  while BrutoCleanupTop <> Frame^.Cleanup do
  begin
    Rec := BrutoCleanupTop;
    for I := 0 to Rec^.Count - 1 do
    begin
      Slot := PPointer(PtrUInt(Rec) + SizeOf(TBrutoCleanup) + PtrUInt(I) * SizeOf(Pointer))^;
      bruto_astr_release(PPointer(Slot)^);
      PPointer(Slot)^ := nil;
    end;
    BrutoCleanupTop := Rec^.Prev;
  end;
  bruto_longjmp(@Frame^.Buf, 1);
end;

procedure BrutoFreeException(Obj: TObject);
begin
  if BrutoExcObject = Obj then
    BrutoExcObject := nil;
  Obj.Free;
end;

type
  PBrutoClassHeader = ^TBrutoClassHeader;
  TBrutoClassHeader = record
    InstanceSize: SizeInt;
    NegInstanceSize: SizeInt;
    ParentRef: PPointer;
    ClassName: PShortString;
    Cleanup: Pointer;
  end;

constructor TObject.Create;
begin
end;

destructor TObject.Destroy;
begin
end;

procedure TObject.Free;
begin
  if Self <> nil then
    Destroy;
end;

class function TObject.ClassName: ShortString;
begin
  ClassName := PBrutoClassHeader(Self)^.ClassName^;
end;

class function TObject.ClassNameIs(const Name: AnsiString): Boolean;
var
  Own: ShortString;
  I: Integer;
begin
  Own := ClassName;
  ClassNameIs := Length(Own) = Length(Name);
  I := 1;
  while ClassNameIs and (I <= Length(Name)) do
  begin
    ClassNameIs := UpCase(Own[I]) = UpCase(Name[I]);
    I := I + 1;
  end;
end;

class function TObject.ClassParent: TClass;
var
  Ref: PPointer;
begin
  Ref := PBrutoClassHeader(Self)^.ParentRef;
  if Ref = nil then
    ClassParent := nil
  else
    ClassParent := TClass(Ref^);
end;

class function TObject.InstanceSize: SizeInt;
begin
  InstanceSize := PBrutoClassHeader(Self)^.InstanceSize;
end;

class function TObject.InheritsFrom(AClass: TClass): Boolean;
var
  C: TClass;
begin
  C := Self;
  while (C <> nil) and (C <> AClass) do
    C := C.ClassParent;
  InheritsFrom := C <> nil;
end;

function TObject.ClassType: TClass;
begin
  ClassType := TClass(PPointer(Self)^);
end;

procedure TObject.AfterConstruction;
begin
end;

procedure TObject.BeforeDestruction;
begin
end;

function TObject.Equals(Obj: TObject): Boolean;
begin
  Equals := Obj = Self;
end;

function TObject.GetHashCode: PtrInt;
begin
  GetHashCode := PtrInt(Self);
end;

function TObject.ToString: AnsiString;
begin
  ToString := ClassName;
end;

end.
