{ Bruto Pascal runtime: the part of FPC's Unix unit Free Vision uses. }
unit Unix;

interface

uses UnixType;

function FpGetTimeOfDay(Tp: PTimeVal; Tzp: Pointer): cint; cdecl; external 'c' name 'gettimeofday';

implementation

end.
