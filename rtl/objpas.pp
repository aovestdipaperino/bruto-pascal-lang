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

implementation

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
